// What the shell's assertive live region reads for a ringing call. The region is always there (so
// its first change is read out); the call alerts, which load with the first call, write into it.

const announcement = $state({ text: "" });

export function incomingCallAnnouncement(): string {
  return announcement.text;
}

export function announceIncomingCall(text: string): void {
  announcement.text = text;
}
