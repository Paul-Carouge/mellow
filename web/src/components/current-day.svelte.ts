// Today, renewed at local midnight: message rows kept from one event to the next still move from
// "Today at" to "Yesterday at" when the day changes.

const clock = $state({ day: new Date() });

function renewAtNextMidnight(): void {
  const now = new Date();
  const nextMidnight = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  setTimeout(() => {
    clock.day = new Date();
    renewAtNextMidnight();
  }, nextMidnight.getTime() - now.getTime());
}

renewAtNextMidnight();

/** Only its day is meant to be read: its time is when it was last renewed. */
export function currentDay(): Date {
  return clock.day;
}
