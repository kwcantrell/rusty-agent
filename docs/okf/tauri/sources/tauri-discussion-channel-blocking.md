---
type: Source
title: "How can I make Channel::send non-blocking? · tauri-apps · Discussion #11589 · GitHub"
description: "User-reported Channel::send blocking behavior (30-50ms) when transmitting large video frame data."
resource: https://github.com/tauri-apps/tauri/discussions/11589
tags: [ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---
# Summary

A user reported that Tauri's `Channel::send` function blocks the calling thread for 30-50ms when transmitting approximately 3MB of video frame data (width, height, and YUV420P format). This blocking behavior disrupts the tokio asynchronous executor in a pipeline that receives h264 data, decodes it to YUV420P format, and sends it to the frontend for canvas rendering.

## User Resolution

The discussion contains only a self-answer from the question author (not a maintainer response). The proposed resolution was: "mark &'static [u8]" — using static lifetime references for the byte data rather than borrowed references to optimize the transmission path.

**Note**: This discussion lacks official maintainer commentary on the blocking behavior or confirmation of the proposed solution's effectiveness.
