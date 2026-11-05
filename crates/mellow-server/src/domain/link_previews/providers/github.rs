//! GitHub repositories, issues and pull requests (ADR-0044), read from the REST API without a
//! token: 60 requests an hour per instance. When GitHub says the budget is spent, no request is
//! made until its reset, and links show the page's card meanwhile.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mellow_protocol::{GitHubIssueEmbed, GitHubIssueState, GitHubRepositoryEmbed, LinkEmbed};
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::Deserialize;
use url::Url;

use super::{ProviderOutcome, ProviderPreview};
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::link_previews::fetcher::{FetchError, LinkFetcher};
use crate::domain::link_previews::page::{
    MAXIMUM_DESCRIPTION_CHARACTERS, MAXIMUM_TITLE_CHARACTERS, PageDetails, plain_text,
};

const GITHUB_ORIGIN: &str = "https://github.com/";
const GITHUB_API_ORIGIN: &str = "https://api.github.com/";
const GITHUB_HOSTS: [&str; 2] = ["github.com", "www.github.com"];
const SITE_NAME: &str = "GitHub";
const MAXIMUM_OWNER_LENGTH: usize = 39;
const MAXIMUM_REPOSITORY_NAME_LENGTH: usize = 100;
const MAXIMUM_NUMBER_DIGITS: usize = 10;
const ISSUES_PATH_KIND: &str = "issues";
const PULL_REQUEST_PATH_KIND: &str = "pull";
/// Tabs of a pull request's page, which name the same pull request.
const PULL_REQUEST_TABS: [&str; 3] = ["files", "commits", "checks"];
const CLONE_SUFFIX: &str = ".git";
/// The card shows the first five; the page has them all.
const MAXIMUM_TOPICS: usize = 5;
const MAXIMUM_TOPIC_CHARACTERS: usize = 50;
const MAXIMUM_LANGUAGE_CHARACTERS: usize = 64;
const MAXIMUM_LICENCE_CHARACTERS: usize = 64;
/// Bots' logins (`dependabot[bot]`) are longer than people's.
const MAXIMUM_LOGIN_CHARACTERS: usize = 64;
/// The excerpt shows 350 characters; links' addresses take room before they are dropped.
const MAXIMUM_BODY_BYTES_READ: usize = 4 * 1024;
/// GitHub's licence id when the licence is none it knows.
const UNKNOWN_LICENCE_SPDX: &str = "NOASSERTION";
/// Twice the card's 40 px avatar.
const AVATAR_SIZE_PIXELS: &str = "80";
/// `x-ratelimit-remaining` and `x-ratelimit-reset` (Unix seconds), GitHub's budget headers.
const RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";
const RATE_LIMIT_RESET: &str = "x-ratelimit-reset";
/// The pause when GitHub refuses without saying until when (its documented minimum).
const DEFAULT_PAUSE_MILLISECONDS: u64 = 60 * 1_000;
/// GitHub's budget is per hour: a reset further away than this is not believed.
const MAXIMUM_PAUSE_MILLISECONDS: u64 = 60 * 60 * 1_000;
const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// A GitHub link the card knows, in its canonical form's parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitHubLink {
    Repository {
        owner: String,
        name: String,
    },
    /// An issue or a pull request: one API endpoint answers both, and says which it is.
    Issue {
        owner: String,
        repository: String,
        number: u64,
        /// The link says `/pull/`: kept so its canonical form stays the one GitHub shows.
        is_pull_request_link: bool,
    },
}

/// Where GitHub is, and the guard keeping the instance within its budget.
#[derive(Debug, Clone)]
pub(crate) struct GitHub {
    /// Links on this origin are GitHub's (a test site in tests).
    origin: Url,
    api_origin: Url,
    paused_until: Arc<RateLimitPause>,
}

impl Default for GitHub {
    fn default() -> Self {
        Self {
            // Constant, valid addresses.
            origin: Url::parse(GITHUB_ORIGIN).unwrap_or_else(|_| unreachable!()),
            api_origin: Url::parse(GITHUB_API_ORIGIN).unwrap_or_else(|_| unreachable!()),
            paused_until: Arc::default(),
        }
    }
}

/// Until when no request goes to GitHub's API, in Unix milliseconds; 0 when it may.
#[derive(Debug, Default)]
struct RateLimitPause(AtomicU64);

impl RateLimitPause {
    fn is_paused(&self, now: u64) -> bool {
        now < self.0.load(Ordering::Relaxed)
    }

    fn pause_until(&self, until: u64) {
        self.0.fetch_max(until, Ordering::Relaxed);
    }
}

impl GitHub {
    #[cfg(test)]
    pub(crate) fn served_by_for_tests(origin: &Url) -> Self {
        Self {
            origin: origin.clone(),
            api_origin: origin.clone(),
            paused_until: Arc::default(),
        }
    }

    /// The repository, issue or pull request a canonical link names.
    pub(crate) fn thing_of(&self, link: &Url) -> Option<GitHubLink> {
        if link.origin() != self.origin.origin() {
            return None;
        }
        github_link_of_path(link)
    }

    pub(crate) async fn preview_of(
        &self,
        fetcher: &LinkFetcher,
        link: &GitHubLink,
    ) -> ProviderOutcome {
        let now = now_unix_milliseconds();
        if self.paused_until.is_paused(now) {
            return ProviderOutcome::Failed;
        }
        let path = match link {
            GitHubLink::Repository { owner, name } => format!("repos/{owner}/{name}"),
            GitHubLink::Issue {
                owner,
                repository,
                number,
                ..
            } => format!("repos/{owner}/{repository}/issues/{number}"),
        };
        let Ok(api_url) = self.api_origin.join(&path) else {
            return ProviderOutcome::Failed;
        };
        let preview = match link {
            GitHubLink::Repository { .. } => fetcher
                .fetch_json_and_headers::<ApiRepository>(api_url)
                .await
                .map(|(headers, answer)| (headers, repository_preview(answer))),
            GitHubLink::Issue {
                owner,
                repository,
                number,
                ..
            } => fetcher
                .fetch_json_and_headers::<ApiIssue>(api_url)
                .await
                .map(|(headers, answer)| {
                    (headers, issue_preview(answer, owner, repository, *number))
                }),
        };
        let now = now_unix_milliseconds();
        match preview {
            Ok((headers, preview)) => {
                self.pause_after(None, &headers, now);
                preview.map_or(ProviderOutcome::Failed, |preview| {
                    ProviderOutcome::Found(Box::new(preview))
                })
            }
            Err(FetchError::Status { status, headers }) => {
                self.pause_after(Some(status), &headers, now);
                ProviderOutcome::Failed
            }
            Err(_) => ProviderOutcome::Failed,
        }
    }

    fn pause_after(&self, status: Option<StatusCode>, headers: &HeaderMap, now: u64) {
        if let Some(until) = pause_until_after_answer(status, headers, now) {
            // No link or count: that the budget ran out is enough (logs rule).
            tracing::debug!(
                "GitHub's API budget is spent: its links show the page's card until the reset"
            );
            self.paused_until.pause_until(until);
        }
    }
}

/// Until when to leave GitHub's API alone after an answer (GitHub's rate-limit guide): the reset
/// once the budget is spent, `retry-after` on a refusal that gives one, a minute on a 429 that
/// gives none. A 403 without either is a refusal of that one link, which pauses nothing.
fn pause_until_after_answer(
    status: Option<StatusCode>,
    headers: &HeaderMap,
    now: u64,
) -> Option<u64> {
    let header_number = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
    };
    let latest = now.saturating_add(MAXIMUM_PAUSE_MILLISECONDS);
    if header_number(RATE_LIMIT_REMAINING) == Some(0) {
        let reset = header_number(RATE_LIMIT_RESET)
            .map_or(now.saturating_add(DEFAULT_PAUSE_MILLISECONDS), |seconds| {
                seconds.saturating_mul(MILLISECONDS_PER_SECOND)
            });
        return Some(reset.clamp(now, latest));
    }
    let status = status?;
    if status != StatusCode::FORBIDDEN && status != StatusCode::TOO_MANY_REQUESTS {
        return None;
    }
    if let Some(seconds) = header_number(RETRY_AFTER.as_str()) {
        return Some(
            now.saturating_add(seconds.saturating_mul(MILLISECONDS_PER_SECOND))
                .min(latest),
        );
    }
    (status == StatusCode::TOO_MANY_REQUESTS)
        .then(|| now.saturating_add(DEFAULT_PAUSE_MILLISECONDS))
}

/// `https://github.com/<owner>/<name>` (lowercase, without `.git`), or its `/issues/<n>` or
/// `/pull/<n>` (a pull request's tabs dropped), without query: one preview per thing.
pub(crate) fn canonical_link(link: &Url) -> Option<Url> {
    if !GITHUB_HOSTS.contains(&link.host_str()?) || link.port().is_some() {
        return None;
    }
    let path = match github_link_of_path(link)? {
        GitHubLink::Repository { owner, name } => format!("{owner}/{name}"),
        GitHubLink::Issue {
            owner,
            repository,
            number,
            is_pull_request_link,
        } => {
            let kind = if is_pull_request_link {
                PULL_REQUEST_PATH_KIND
            } else {
                ISSUES_PATH_KIND
            };
            format!("{owner}/{repository}/{kind}/{number}")
        }
    };
    Url::parse(GITHUB_ORIGIN).ok()?.join(&path).ok()
}

fn github_link_of_path(link: &Url) -> Option<GitHubLink> {
    let parts: Vec<&str> = link
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect();
    let (owner, repository, rest) = match parts.as_slice() {
        [owner, repository, rest @ ..] => (*owner, *repository, rest),
        _ => return None,
    };
    let repository = repository.strip_suffix(CLONE_SUFFIX).unwrap_or(repository);
    if !is_owner_login(owner) || !is_repository_name(repository) {
        return None;
    }
    let (owner, repository) = (owner.to_ascii_lowercase(), repository.to_ascii_lowercase());
    let (number, is_pull_request_link) = match rest {
        [] => {
            return Some(GitHubLink::Repository {
                owner,
                name: repository,
            });
        }
        [ISSUES_PATH_KIND, number] => (*number, false),
        [PULL_REQUEST_PATH_KIND, number] => (*number, true),
        [PULL_REQUEST_PATH_KIND, number, tab] if PULL_REQUEST_TABS.contains(tab) => (*number, true),
        _ => return None,
    };
    Some(GitHubLink::Issue {
        owner,
        repository,
        number: issue_number(number)?,
        is_pull_request_link,
    })
}

/// GitHub's logins: letters, digits and hyphens, not leading.
fn is_owner_login(login: &str) -> bool {
    (1..=MAXIMUM_OWNER_LENGTH).contains(&login.len())
        && !login.starts_with('-')
        && login
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// GitHub's repository names: letters, digits, `.`, `-` and `_`, never `.` or `..` alone.
fn is_repository_name(name: &str) -> bool {
    (1..=MAXIMUM_REPOSITORY_NAME_LENGTH).contains(&name.len())
        && name != "."
        && name != ".."
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn issue_number(text: &str) -> Option<u64> {
    if text.is_empty()
        || text.len() > MAXIMUM_NUMBER_DIGITS
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    text.parse().ok().filter(|number| *number > 0)
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    login: String,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiLicence {
    #[serde(default)]
    spdx_id: Option<String>,
}

/// The fields of `GET /repos/{owner}/{repo}` the card shows.
#[derive(Debug, Deserialize)]
struct ApiRepository {
    name: String,
    owner: ApiUser,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    license: Option<ApiLicence>,
    #[serde(default)]
    archived: bool,
}

/// Present on an issue that is a pull request.
#[derive(Debug, Deserialize)]
struct ApiPullRequestPart {
    #[serde(default)]
    merged_at: Option<String>,
}

/// The fields of `GET /repos/{owner}/{repo}/issues/{number}` the card shows.
#[derive(Debug, Deserialize)]
struct ApiIssue {
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    #[serde(default)]
    state_reason: Option<String>,
    #[serde(default)]
    user: Option<ApiUser>,
    #[serde(default)]
    comments: u64,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    pull_request: Option<ApiPullRequestPart>,
    /// `https://api.github.com/repos/<Owner>/<Name>`: the names as their owner writes them.
    #[serde(default)]
    repository_url: Option<String>,
}

fn repository_preview(answer: ApiRepository) -> Option<ProviderPreview> {
    if !is_owner_login(&answer.owner.login) || !is_repository_name(&answer.name) {
        return None;
    }
    let image_url = answer.owner.avatar_url.as_deref().and_then(avatar_url);
    let topics = answer
        .topics
        .iter()
        .filter_map(|topic| plain_text(topic, MAXIMUM_TOPIC_CHARACTERS))
        .take(MAXIMUM_TOPICS)
        .collect();
    let licence_spdx = answer
        .license
        .and_then(|licence| licence.spdx_id)
        .filter(|id| id != UNKNOWN_LICENCE_SPDX)
        .and_then(|id| plain_text(&id, MAXIMUM_LICENCE_CHARACTERS));
    Some(ProviderPreview {
        details: PageDetails {
            title: Some(format!("{}/{}", answer.owner.login, answer.name)),
            description: answer
                .description
                .and_then(|text| plain_text(&text, MAXIMUM_DESCRIPTION_CHARACTERS)),
            site_name: Some(SITE_NAME.to_owned()),
            image_url,
            theme_color: None,
        },
        picture: None,
        embed: LinkEmbed::GitHubRepository(GitHubRepositoryEmbed {
            owner: answer.owner.login,
            name: answer.name,
            stars: answer.stargazers_count,
            forks: answer.forks_count,
            language: answer
                .language
                .and_then(|language| plain_text(&language, MAXIMUM_LANGUAGE_CHARACTERS)),
            // The web client colours the language from its own table.
            language_color: None,
            topics,
            licence_spdx,
            is_archived: answer.archived,
        }),
    })
}

/// The avatar at the card's size; GitHub's avatar host takes `s=<pixels>`.
fn avatar_url(link: &str) -> Option<Url> {
    let mut url = Url::parse(link)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))?;
    url.query_pairs_mut().append_pair("s", AVATAR_SIZE_PIXELS);
    Some(url)
}

/// `owner` and `repository` come from the link; the answer's own spelling is preferred.
fn issue_preview(
    answer: ApiIssue,
    owner: &str,
    repository: &str,
    number: u64,
) -> Option<ProviderPreview> {
    let (owner, repository) = answer
        .repository_url
        .as_deref()
        .and_then(owner_and_repository_of_api_url)
        .filter(|(answered_owner, answered_repository)| {
            answered_owner.eq_ignore_ascii_case(owner)
                && answered_repository.eq_ignore_ascii_case(repository)
        })
        .unwrap_or_else(|| (owner.to_owned(), repository.to_owned()));
    let is_pull_request = answer.pull_request.is_some();
    let merged_at = answer
        .pull_request
        .as_ref()
        .and_then(|pull_request| pull_request.merged_at.as_deref())
        .and_then(unix_milliseconds_of_timestamp);
    let state = match (answer.state.as_str(), is_pull_request) {
        _ if merged_at.is_some() => GitHubIssueState::Merged,
        ("open", true) if answer.draft => GitHubIssueState::Draft,
        ("open", _) => GitHubIssueState::Open,
        ("closed", false) => match answer.state_reason.as_deref() {
            Some("completed") => GitHubIssueState::Completed,
            Some("not_planned") => GitHubIssueState::NotPlanned,
            _ => GitHubIssueState::Closed,
        },
        ("closed", true) => GitHubIssueState::Closed,
        _ => return None,
    };
    let author = answer
        .user
        .and_then(|user| plain_text(&user.login, MAXIMUM_LOGIN_CHARACTERS))
        .unwrap_or_default();
    let embed = GitHubIssueEmbed {
        owner,
        repository,
        number,
        state,
        author,
        opened_at_unix_milliseconds: answer
            .created_at
            .as_deref()
            .and_then(unix_milliseconds_of_timestamp),
        merged_at_unix_milliseconds: merged_at,
        comments: u32::try_from(answer.comments).unwrap_or(u32::MAX),
    };
    Some(ProviderPreview {
        details: PageDetails {
            title: plain_text(&answer.title, MAXIMUM_TITLE_CHARACTERS),
            description: answer.body.as_deref().and_then(body_excerpt),
            site_name: Some(SITE_NAME.to_owned()),
            // The author's avatar is not shown for issues (docs/design/web-provider-embeds.md).
            image_url: None,
            theme_color: None,
        },
        picture: None,
        embed: if is_pull_request {
            LinkEmbed::GitHubPullRequest(embed)
        } else {
            LinkEmbed::GitHubIssue(embed)
        },
    })
}

fn owner_and_repository_of_api_url(link: &str) -> Option<(String, String)> {
    let url = Url::parse(link).ok()?;
    let parts: Vec<&str> = url.path_segments()?.collect();
    match parts.as_slice() {
        [.., "repos", owner, repository]
            if is_owner_login(owner) && is_repository_name(repository) =>
        {
            Some(((*owner).to_owned(), (*repository).to_owned()))
        }
        _ => None,
    }
}

/// The body's opening as one plain line: without the HTML comments of issue templates, heading
/// and bold marks, or links' addresses, which would otherwise be all a reader sees of many pull
/// requests.
fn body_excerpt(body: &str) -> Option<String> {
    let mut text = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        text.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + "-->".len()..]);
    }
    text.push_str(rest);
    let mut opening_end = text.len().min(MAXIMUM_BODY_BYTES_READ);
    while !text.is_char_boundary(opening_end) {
        opening_end -= 1;
    }
    let text = without_markdown_link_targets(&text[..opening_end]).replace("**", "");
    let lines: Vec<&str> = text
        .lines()
        .map(|line| line.trim_start().trim_start_matches('#'))
        .collect();
    plain_text(&lines.join("\n"), MAXIMUM_DESCRIPTION_CHARACTERS)
}

/// `[label](address)` and `![label](address)` as their label. Each `[` searches the rest of the
/// text, so the text is cut to `MAXIMUM_BODY_BYTES_READ` first.
fn without_markdown_link_targets(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after_open = &rest[open + 1..];
        let link = after_open.find("](").and_then(|close| {
            let target_end = after_open[close + 2..].find(')')?;
            (!after_open[..close].contains('[')).then_some((close, close + 2 + target_end + 1))
        });
        if let Some((label_end, link_end)) = link {
            kept.push_str(rest[..open].strip_suffix('!').unwrap_or(&rest[..open]));
            kept.push_str(&after_open[..label_end]);
            rest = &after_open[link_end..];
        } else {
            kept.push_str(&rest[..=open]);
            rest = after_open;
        }
    }
    kept.push_str(rest);
    kept
}

/// GitHub's `2026-11-03T10:00:00Z` (UTC, whole seconds) as Unix milliseconds.
fn unix_milliseconds_of_timestamp(timestamp: &str) -> Option<u64> {
    let bytes = timestamp.as_bytes();
    if bytes.len() != "YYYY-MM-DDTHH:MM:SSZ".len()
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u64> {
        let digits = timestamp.get(range)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())
            .flatten()
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_since_unix_epoch(year, month, day);
    let seconds = ((days * 24 + hour) * 60 + minute) * 60 + second;
    seconds.checked_mul(MILLISECONDS_PER_SECOND)
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar from 1970 on (Howard
/// Hinnant's `days_from_civil`).
fn days_since_unix_epoch(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    // 719 468 days from 0000-03-01 to 1970-01-01; the year is at least 1970, so this is positive.
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn canonical(link: &str) -> Option<String> {
        canonical_link(&Url::parse(link).unwrap()).map(|url| url.to_string())
    }

    #[test]
    fn repository_issue_and_pull_request_links_share_one_form_each() {
        for link in [
            "https://github.com/Rust-Lang/Rust",
            "https://www.github.com/rust-lang/rust/",
            "https://github.com/rust-lang/rust.git",
            "https://github.com/rust-lang/rust?tab=readme-ov-file",
        ] {
            assert_eq!(
                canonical(link).as_deref(),
                Some("https://github.com/rust-lang/rust"),
                "{link}"
            );
        }
        assert_eq!(
            canonical("https://github.com/rust-lang/rust/issues/12345?q=x").as_deref(),
            Some("https://github.com/rust-lang/rust/issues/12345")
        );
        for link in [
            "https://github.com/rust-lang/rust/pull/678",
            "https://github.com/rust-lang/rust/pull/678/files",
            "https://github.com/rust-lang/rust/pull/678/commits?w=1",
        ] {
            assert_eq!(
                canonical(link).as_deref(),
                Some("https://github.com/rust-lang/rust/pull/678"),
                "{link}"
            );
        }
    }

    #[test]
    fn other_github_pages_and_look_alikes_are_left_alone() {
        for link in [
            "https://github.com/rust-lang",
            "https://github.com/rust-lang/rust/tree/master/src",
            "https://github.com/rust-lang/rust/blob/master/README.md",
            "https://github.com/rust-lang/rust/issues",
            "https://github.com/rust-lang/rust/issues/0",
            "https://github.com/rust-lang/rust/issues/12a",
            "https://github.com/rust-lang/rust/issues/12345678901",
            "https://github.com/rust-lang/rust/pull/1/conflicts",
            "https://github.com/-lead/rust",
            "https://github.com/rust-lang/..",
            "https://github.com/rust%20lang/rust",
            "https://github.com:8443/rust-lang/rust",
            "https://gist.github.com/rust-lang/rust",
            "https://github.com.evil.example/rust-lang/rust",
        ] {
            assert_eq!(canonical(link), None, "{link}");
        }
    }

    #[test]
    fn github_timestamps_are_read_as_utc() {
        assert_eq!(
            unix_milliseconds_of_timestamp("1970-01-01T00:00:00Z"),
            Some(0)
        );
        assert_eq!(
            unix_milliseconds_of_timestamp("2026-11-03T10:00:00Z"),
            Some(1_793_700_000_000)
        );
        assert_eq!(
            unix_milliseconds_of_timestamp("2024-02-29T23:59:59Z"),
            Some(1_709_251_199_000)
        );
        for malformed in [
            "2026-11-03",
            "2026-11-03T10:00:00+01:00",
            "2026-13-03T10:00:00Z",
            "2026-11-03T24:00:00Z",
            "+026-11-03T10:00:00Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert_eq!(
                unix_milliseconds_of_timestamp(malformed),
                None,
                "{malformed}"
            );
        }
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn the_api_is_left_alone_until_github_says_it_takes_requests_again() {
        let now = 1_000_000_000;
        let spent = headers(&[(RATE_LIMIT_REMAINING, "0"), (RATE_LIMIT_RESET, "1000600")]);
        assert_eq!(
            pause_until_after_answer(Some(StatusCode::FORBIDDEN), &spent, now),
            Some(1_000_600_000)
        );
        assert_eq!(
            pause_until_after_answer(None, &spent, now),
            Some(1_000_600_000),
            "the last request of the budget pauses too"
        );
        let far = headers(&[
            (RATE_LIMIT_REMAINING, "0"),
            (RATE_LIMIT_RESET, "9999999999"),
        ]);
        assert_eq!(
            pause_until_after_answer(Some(StatusCode::FORBIDDEN), &far, now),
            Some(now + MAXIMUM_PAUSE_MILLISECONDS)
        );
        let retry = headers(&[(RATE_LIMIT_REMAINING, "12"), ("retry-after", "30")]);
        assert_eq!(
            pause_until_after_answer(Some(StatusCode::TOO_MANY_REQUESTS), &retry, now),
            Some(now + 30_000)
        );
        assert_eq!(
            pause_until_after_answer(Some(StatusCode::TOO_MANY_REQUESTS), &HeaderMap::new(), now),
            Some(now + DEFAULT_PAUSE_MILLISECONDS)
        );
        let left = headers(&[(RATE_LIMIT_REMAINING, "59"), (RATE_LIMIT_RESET, "1000600")]);
        assert_eq!(pause_until_after_answer(None, &left, now), None);
        assert_eq!(
            pause_until_after_answer(Some(StatusCode::FORBIDDEN), &left, now),
            None,
            "a blocked repository pauses nothing"
        );
        assert_eq!(
            pause_until_after_answer(Some(StatusCode::NOT_FOUND), &HeaderMap::new(), now),
            None
        );
    }

    #[test]
    fn issue_templates_comments_and_heading_marks_are_left_out_of_the_excerpt() {
        assert_eq!(
            body_excerpt("<!-- Thanks for your pull request! -->\n## Summary\nStabilise `async || {}`.\n<!-- unclosed").as_deref(),
            Some("Summary Stabilise `async || {}`.")
        );
        assert_eq!(body_excerpt("<!-- only a template -->\n"), None);
        assert_eq!(
            body_excerpt("This **stabilises** ([RFC 3668](https://rust-lang.github.io/rfcs/3668.html)) ![logo](x.png) [x] done [a [b](c)").as_deref(),
            Some("This stabilises (RFC 3668) logo [x] done [a b")
        );
        let long_body = format!("{}é tail", "a".repeat(MAXIMUM_BODY_BYTES_READ - 1));
        assert!(
            body_excerpt(&long_body).is_some(),
            "cut on a character boundary"
        );
    }
}
