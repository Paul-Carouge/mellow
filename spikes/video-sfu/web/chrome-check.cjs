// PAGES (default 2) Chromium pages send a synthetic simulcast camera to the spike SFU and watch each other,
// then one page measures the size of offers with video m-lines.
// Usage: [PAGES=3] [PLAYWRIGHT=/path/to/node_modules/playwright] node web/chrome-check.cjs http://IP:3000
const { chromium } = require(process.env.PLAYWRIGHT || "playwright");

(async () => {
  const url = process.argv[2];
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const pages = [];
  for (let i = 0; i < Number(process.env.PAGES || 2); i += 1) pages.push(await context.newPage());
  for (const page of pages) {
    await page.goto(url);
    await page.click("#canvas");
  }
  await new Promise((r) => setTimeout(r, 15000));
  for (const page of pages) {
    console.log("---");
    console.log(await page.textContent("#stats"));
  }
  // Offer sizes: one audio send + 16 audio receive (today), plus one simulcast camera and 9 video
  // receive m-lines, with every codec, VP8 only, or VP8 and Opus only.
  const sizes = await pages[0].evaluate(async () => {
    async function offerBytes(restrictToVp8, videoSlots = 9, restrictAudio = false) {
      const connection = new RTCPeerConnection();
      connection.addTransceiver("audio", { direction: "sendonly" });
      for (let i = 0; i < 16; i += 1) connection.addTransceiver("audio", { direction: "recvonly" });
      const canvas = document.createElement("canvas");
      const [track] = canvas.captureStream(30).getVideoTracks();
      const video = [connection.addTransceiver(track, {
        direction: "sendonly",
        sendEncodings: [{ rid: "q", scaleResolutionDownBy: 4 }, { rid: "l", scaleResolutionDownBy: 2 }, { rid: "h" }],
      })];
      for (let i = 0; i < videoSlots; i += 1) video.push(connection.addTransceiver("video", { direction: "recvonly" }));
      if (restrictToVp8) {
        const codecs = RTCRtpReceiver.getCapabilities("video").codecs
          .filter((c) => c.mimeType === "video/VP8" || c.mimeType === "video/rtx");
        for (const transceiver of video) transceiver.setCodecPreferences(codecs);
      }
      if (restrictAudio) {
        const opus = RTCRtpReceiver.getCapabilities("audio").codecs.filter((c) => c.mimeType === "audio/opus");
        for (const t of connection.getTransceivers()) if (t.receiver.track.kind === "audio") t.setCodecPreferences(opus);
      }
      await connection.setLocalDescription();
      await new Promise((resolve) => {
        if (connection.iceGatheringState === "complete") return resolve();
        connection.onicegatheringstatechange = () => connection.iceGatheringState === "complete" && resolve();
      });
      const bytes = connection.localDescription.sdp.length;
      connection.close();
      return bytes;
    }
    return { everyCodec: await offerBytes(false), vp8Only: await offerBytes(true), vp8AndOpusOnly: await offerBytes(true, 9, true), vp8AndOpusOnly4Slots: await offerBytes(true, 4, true), cameraWithoutVideoSlots: await offerBytes(false, 0) };
  });
  console.log("offer bytes", JSON.stringify(sizes), await browser.version());
  await browser.close();
})();
