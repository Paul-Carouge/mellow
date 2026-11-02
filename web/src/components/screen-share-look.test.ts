import { describe, expect, it } from "vitest";

import type { VoiceState } from "../lib/protocol";
import { liveBadgeOf, watchedStreamStatus } from "./screen-share-look";

const ME = "1";
const ANA = "2";
const SALON = "21";
const JEUX = "22";

function voiceState(userId: string, channelId: string, extra: Partial<VoiceState> = {}): VoiceState {
  return { user_id: userId, guild_id: "100", channel_id: channelId, self_mute: false, self_deaf: false, suppressed: false, ...extra };
}

describe("liveBadgeOf", () => {
  const inSalon = { connectedChannelId: SALON, canWatchStreams: true };

  it("shows nothing for someone who does not share, whatever older servers leave out", () => {
    expect(liveBadgeOf(voiceState(ANA, SALON), ME, inSalon)).toBeNull();
    expect(liveBadgeOf(voiceState(ANA, SALON, { self_stream: false, self_video: true }), ME, inSalon)).toBeNull();
  });

  it("lets you watch someone sharing in your call", () => {
    expect(liveBadgeOf(voiceState(ANA, SALON, { self_stream: true }), ME, inSalon)).toBe("watch");
  });

  it("shows your own share without opening it", () => {
    expect(liveBadgeOf(voiceState(ME, SALON, { self_stream: true }), ME, inSalon)).toBe("own");
  });

  it("asks to join first for a share in another channel, or before your call is connected", () => {
    expect(liveBadgeOf(voiceState(ANA, JEUX, { self_stream: true }), ME, inSalon)).toBe("join-to-watch");
    expect(
      liveBadgeOf(voiceState(ANA, SALON, { self_stream: true }), ME, { connectedChannelId: null, canWatchStreams: false }),
    ).toBe("join-to-watch");
  });

  it("says so when this browser joined the call without video", () => {
    expect(
      liveBadgeOf(voiceState(ANA, SALON, { self_stream: true }), ME, { connectedChannelId: SALON, canWatchStreams: false }),
    ).toBe("cannot-watch");
  });
});

describe("watchedStreamStatus", () => {
  it("connects until the picture's track arrives, then is live", () => {
    const sharing = voiceState(ANA, SALON, { self_stream: true });
    expect(watchedStreamStatus(sharing, SALON, false)).toBe("connecting");
    expect(watchedStreamStatus(sharing, SALON, true)).toBe("live");
  });

  it("ends when they stop sharing, even if the last track is still there", () => {
    expect(watchedStreamStatus(voiceState(ANA, SALON, { self_stream: false }), SALON, true)).toBe("ended");
    expect(watchedStreamStatus(voiceState(ANA, SALON), SALON, true)).toBe("ended");
  });

  it("ends when they leave or move, or when you leave the call", () => {
    expect(watchedStreamStatus(undefined, SALON, false)).toBe("ended");
    expect(watchedStreamStatus(voiceState(ANA, JEUX, { self_stream: true }), SALON, true)).toBe("ended");
    expect(watchedStreamStatus(voiceState(ANA, SALON, { self_stream: true }), null, true)).toBe("ended");
  });
});
