# Lane: voice
## Scope
- src/voice/mod.rs (77 LOC)
- src/voice/pipeline.rs (300 LOC)

## Findings
### [P0] STT transcribe silently drops all audio frames after the first frame
- Location: src/voice/pipeline.rs:149
- Evidence: let Some(first) = frames.first() else {
- Why it matters: `SpeechPipeline::process` receives a slice of `AudioFrame`s representing an utterance or buffered audio and passes them to `OpenAiSpeechToText::transcribe`. The method only extracts `first = frames.first()` and ignores `frames[1..]`. For any speech segment longer than a single 20ms frame (such as a 5-second sentence consisting of 250 frames), 99.6% of the speech audio is silently dropped, sending only the initial 20ms slice (usually silence or ambient noise) to OpenAI Whisper and resulting in empty or corrupted transcripts.
- Suggested fix: When handling PCM frames, concatenate the sample buffers across all frames in `frames` (verifying matching sample rate and channel count) into the WAV data payload rather than only reading `first`.

### [P0] RTP packet slicing indexes raw packet with payload offset, corrupting Opus audio
- Location: src/voice/mod.rs:67
- Evidence: packet.packet[packet.payload_offset..end].to_vec(),
- Why it matters: In Songbird's `RtpData`, `packet.packet` is the raw UDP packet buffer including the fixed 12-byte RTP header (and optional CSRC list/extensions), while `packet.payload_offset` is the offset within `packet.rtp().payload()`. Slicing `packet.packet[packet.payload_offset..end]` starts at the RTP header bytes instead of the Opus audio payload. Every packet received via `EventContext::RtpPacket` is corrupted with prepended RTP header data, causing downstream Opus decoders to fail or reject frames as invalid bitstreams.
- Suggested fix: Extract the payload slice via `packet.rtp().payload()` and index into that slice: `payload[packet.payload_offset..payload.len().saturating_sub(packet.payload_end_pad)].to_vec()`.

### [P0] VoiceAudioPipeline::channel panics on zero capacity
- Location: src/voice/pipeline.rs:278
- Evidence: pub fn channel(capacity: usize) -> Self {
- Why it matters: `tokio::sync::mpsc::channel(capacity)` panics at runtime if `capacity == 0` with `"mpsc::channel: capacity must be greater than 0"`. Unlike `AudioFrameBuffer::new(0)` which safely supports zero capacity, calling `VoiceAudioPipeline::channel(0)` causes an unhandled panic that crashes the gateway process.
- Suggested fix: Enforce a minimum capacity with `capacity.max(1)` or return a `Result` that rejects zero capacity.

### [P1] SongbirdAudioEventListener fails to unregister on receiver drop, leaking handlers and CPU
- Location: src/voice/mod.rs:49
- Evidence: if self.sender.send(frame).await.is_err() {
- Why it matters: When the downstream receiver is dropped (e.g. voice connection disconnects or consumer task stops), `self.sender.send(frame).await` fails. In `VoiceTick` the loop breaks, and in `RtpPacket` the error is ignored, and `act()` returns `None`. In Songbird's event system, returning `None` keeps the handler active; only returning `Some(Event::Cancel)` unregisters it. Consequently, Songbird retains the dead listener in its event store forever, repeatedly executing `act()`, cloning buffers, allocating frames, and attempting sends to a closed channel every 20ms.
- Suggested fix: Return `Some(songbird::events::Event::Cancel)` whenever `self.sender.send(...).await.is_err()` so Songbird immediately unregisters and drops the listener.

### [P1] Awaiting bounded channel send inside SongbirdAudioEventListener stalls Songbird central event loop
- Location: src/voice/mod.rs:70
- Evidence: let _ = self.sender.send(frame).await;
- Why it matters: Songbird invokes `EventHandler::act` sequentially on a single central async event task (`songbird::driver::tasks::events`). Awaiting `self.sender.send(frame).await` on a bounded mpsc channel causes Songbird's entire event dispatch loop to block whenever downstream processing lags or channel capacity fills. While blocked, all other events (track events, speaking state updates, driver events) across the gateway are stalled.
- Suggested fix: Use `self.sender.try_send(frame)` instead of `.send().await`, dropping overflow frames and recording a backpressure metric rather than stalling the driver event task.

### [P1] VoiceAudioPipeline::receive holds Mutex lock across unbounded recv await
- Location: src/voice/pipeline.rs:298
- Evidence: self.receiver.lock().await.recv().await
- Why it matters: `self.receiver.lock().await` returns a `MutexGuard` whose lifetime spans the entire statement, holding the lock while awaiting `.recv().await`. If no audio frames are available, `.recv().await` waits indefinitely while holding the mutex lock. Because `VoiceAudioPipeline` derives `Clone`, any other worker task attempting to call `receive()` or inspect the receiver is blocked on the mutex lock, serializing all consumers and preventing concurrent processing.
- Suggested fix: Use a dedicated single-consumer reader task or a multi-producer multi-consumer channel rather than wrapping a single `mpsc::Receiver` in `Arc<Mutex<_>>` and holding the lock across `.await`.

### [P1] Raw Opus payload submitted as audio.ogg causes OpenAI API rejection
- Location: src/voice/pipeline.rs:153
- Evidence: AudioPayload::Opus(opus_bytes) => (opus_bytes.clone(), "audio.ogg"),
- Why it matters: An `AudioPayload::Opus` produced by `SongbirdAudioEventListener` contains raw Opus packet bytes without Ogg encapsulation (no `OggS` pages, OpusHead, or OpusTags headers). Submitting raw Opus frames with filename `"audio.ogg"` causes OpenAI's `/audio/transcriptions` API to reject the request with HTTP 400 Bad Request because the payload is not a valid Ogg bitstream.
- Suggested fix: Transcode Opus frames to WAV/PCM before sending or wrap them in an Ogg container structure before submitting multipart form data.

### [P1] Strict equality check in AudioFrameBuffer::push risks unbounded memory growth
- Location: src/voice/pipeline.rs:82
- Evidence: let evicted = (self.frames.len() == self.capacity)
- Why it matters: The buffer checks `self.frames.len() == self.capacity` to decide whether to evict the oldest frame. If `self.frames.len()` is ever greater than `self.capacity` (e.g. if capacity is reduced or the buffer is deserialized from JSON), `len == capacity` is false, eviction never triggers, and subsequent calls to `push` will grow `VecDeque` indefinitely without bound.
- Suggested fix: Replace `self.frames.len() == self.capacity` with `self.frames.len() >= self.capacity`.

### [P1] Unsynchronized multi-speaker interleaving into single audio stream
- Location: src/voice/mod.rs:41
- Evidence: for (source_id, voice) in &tick.speaking {
- Why it matters: `tick.speaking` contains all active speakers during a 20ms tick in a `HashMap<u32, VoiceData>`. The loop iterates through them with nondeterministic hash map ordering and pushes each user's PCM frame into the same channel with shared sequence numbers. Downstream receives interleaved 20ms fragments from different speakers mixed into a single sequence, with no stream demuxing or mixing, producing corrupted speech input for STT.
- Suggested fix: Demux frames into per-SSRC queues or mix the PCM samples of all concurrent speakers into a single combined PCM frame before sending.

### [P2] Missing request timeout on OpenAiSpeechToText HTTP client
- Location: src/voice/pipeline.rs:141
- Evidence: client: reqwest::Client::new(),
- Why it matters: `reqwest::Client::new()` defaults to no request timeout. If the OpenAI endpoint hangs or connection drops without a TCP RST, the HTTP request will block the worker task indefinitely.
- Suggested fix: Set an explicit timeout on the client builder (e.g. `reqwest::Client::builder().timeout(Duration::from_secs(30)).build()`).

### [P2] Inefficient unreserved Vec growth in transcribe WAV generator
- Location: src/voice/pipeline.rs:155
- Evidence: let mut wav = Vec::new();
- Why it matters: `wav` is initialized with zero capacity and then extended 2 bytes at a time in a loop over `pcm_samples`. For large audio frames, this triggers dozens of reallocations and memcpys on the async worker thread.
- Suggested fix: Preallocate the vector with `Vec::with_capacity(44 + pcm_samples.len() * 2)`.

### [P2] SpeechPipeline::process invokes LLM and TTS on empty silent audio transcripts
- Location: src/voice/pipeline.rs:258
- Evidence: let response = self.llm.respond(channel_id, &transcript).await?;
- Why it matters: When audio frames contain silence or STT produces an empty transcript, `process` proceeds to invoke `self.llm.respond(channel_id, "")` and synthesize audio for the LLM response, wasting tokens and producing unwanted bot audio for empty intervals.
- Suggested fix: Check `if transcript.trim().is_empty()` and return early with an empty `SpeechPipelineOutput`.

## Strengths
- Clean separation of concerns between `SpeechToText`, `VoiceLanguageModel`, and `TextToSpeech` traits, making components modular and testable.
- Clean `AudioFrame` data model supporting both Opus and PCM representations, with sample rate, channels, direction, and source metadata.
- Clean ring buffer semantics in `AudioFrameBuffer` for zero-capacity bypass and FIFO frame eviction.
- Standard RIFF WAV header generation logic for 16-bit PCM samples with correct byte-rate and block-align calculations.

## Notes
- `SongbirdAudioEventListener` handles both `VoiceTick` and `RtpPacket` contexts. If registered for both or when Songbird decode mode is enabled, the pipeline can receive duplicate frames (both decoded PCM and raw Opus) for the same audio stream.
- Error taxonomy: `src/error.rs` lacks a dedicated `Voice` variant in `OmonError`, forcing pipeline components to map voice stream and STT errors to `OmonError::Config` and `OmonError::Multiplexer`.
