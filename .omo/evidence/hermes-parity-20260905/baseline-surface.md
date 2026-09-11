# Baseline real binary / real OMO surface

Baseline source prior to parity implementation.

## Health

HTTP/1.1 200 OK
content-type: application/json
content-length: 72
date: Sat, 05 Sep 2026 08:57:18 GMT

{"status":"ok","time":"2026-09-05T08:57:18.500358Z","uptime_seconds":25}

## Chat ingress

HTTP/1.1 202 Accepted
content-type: application/json
content-length: 69
date: Sat, 05 Sep 2026 08:57:36 GMT

{"queued":true,"session_id":"3:web|-|13:hermes-parity|-|9:dashboard"}

## WebSocket completion

{
  "event": {
    "chunk": {
      "content": "HERMES-PARITY-OK",
      "is_final": true,
      "sequence": 4,
      "stream_id": "3993592a-c367-4ef9-9afb-cfe97541b65a"
    },
    "session": {
      "bot_id": null,
      "channel_id": "hermes-parity",
      "guild_id": null,
      "platform": "web",
      "thread_id": null,
      "user_id": "dashboard"
    },
    "type": "stream"
  },
  "type": "event"
}

## Transcript HTTP

HTTP/1.1 200 OK
content-type: application/json
content-length: 391
date: Sat, 05 Sep 2026 08:57:46 GMT

{"items":[{"content":"Reply exactly HERMES-PARITY-OK","created_at":"2026-09-05T08:57:36.722728+00:00","id":"a2b6db1a-6514-4e2d-83a8-73a58f649c63","metadata":[],"role":"user","sequence":1},{"content":"HERMES-PARITY-OK","created_at":"2026-09-05T08:57:46.311496+00:00","id":"c463f656-e5a4-480f-a810-0cd3ecf0588d","metadata":{},"role":"assistant","sequence":2}],"page":1,"per_page":50,"total":2}

## SQLite

[{"role":"user","content":"Reply exactly HERMES-PARITY-OK"},
{"role":"assistant","content":"HERMES-PARITY-OK"}]
[{"state_json":"{\"active_model\":null,\"system_prompt\":null,\"enabled_toolsets\":null,\"yolo\":false,\"suspended\":false,\"metadata\":{\"omo_thread_id\":\"01a070c9-9edd-70b0-9d87-e5bb96e4ce68\"}}"}]

## Startup stack evidence

status: completed exit_code: 0
Sampling process 5599 for 1 second with 1 millisecond of run time between samples
Couldn't find _sigtramp symbol in expected dylibs
Sampling completed, processing symbols...
Sample analysis of process 5599 written to file /tmp/omo-gateway_2026-09-05_175634_IyAn.sample.txt

Analysis of sampling omo-gateway (pid 5599) every 1 millisecond
Process:         omo-gateway [5599]
Path:            /Users/USER/*/omo-gateway
Load Address:    0x1026d8000
Identifier:      omo-gateway
Version:         ???
Code Type:       ARM64
Parent Process:  bash [5597]
Target Type:     live task

Date/Time:       2026-09-05 17:56:34.471 +0900
Launch Time:     2026-09-05 17:55:09.028 +0900
OS Version:      macOS 26.6 (25G72)
Report Version:  7
Analysis Tool:   /usr/bin/sample

Physical footprint:         112K
Physical footprint (peak):  112K
Idle exit:                  untracked
----

Call graph:
    890 Thread_12537027: Main Thread   DispatchQueue_<multiple>
      890 _dyld_start  (in dyld) + 0  [0x1082e49c0]

Total number in stack (recursive counted multiple, when >=5):

Sort by top of stack, same collapsed (when >= 5):
        _dyld_start  (in dyld)        890

Binary images description not available

## Invocation

cd '/var/folders/zh/7cc25lt91b1_dj577306nwdh0000gn/T/omo-hermes-parity-qa.Ek95aSaF7X' && OMO_CODING_AGENT_SESSION_DIR='/var/folders/zh/7cc25lt91b1_dj577306nwdh0000gn/T/omo-hermes-parity-qa.Ek95aSaF7X/omo-sessions' PI_TELEMETRY=0 /Users/indo/.bun/bin/omo app-server --listen ws://127.0.0.1:19842 --ws-auth off --json-logs

cd '/var/folders/zh/7cc25lt91b1_dj577306nwdh0000gn/T/omo-hermes-parity-qa.Ek95aSaF7X' && DATABASE_URL='sqlite:///var/folders/zh/7cc25lt91b1_dj577306nwdh0000gn/T/omo-hermes-parity-qa.Ek95aSaF7X/gateway.db' OMON_WORKSPACE_ROOT='/var/folders/zh/7cc25lt91b1_dj577306nwdh0000gn/T/omo-hermes-parity-qa.Ek95aSaF7X/workspace' HERMES_HOME='/var/folders/zh/7cc25lt91b1_dj577306nwdh0000gn/T/omo-hermes-parity-qa.Ek95aSaF7X/hermes' OMON_OMO_APPSERVER_URL='ws://127.0.0.1:19842' OMON_OMO_CRON_APPSERVER_URL='ws://127.0.0.1:19842' OMON_OMO_AUTOSPAWN=off OMON_DEFAULT_MODEL='ocx/gpt-6-astra' DEFAULT_MODEL='ocx/gpt-6-astra' DISCORD_BOT_TOKEN='' DISCORD_BOT_TOKENS='' RUST_LOG=info '/Users/indo/code/project/omon-gateway/target/debug/omo-gateway' dashboard --host 127.0.0.1 --port 19744

## Cleanup

WebSocket closed in finally; gateway/appserver teardown follows.

Cleanup receipt: WebSocket closed; gateway Ctrl+C exited 0, OMO app-server Ctrl+C exited 1; both monitor process trees terminated. Ports 19744 and 19842 had no listeners after teardown. Temporary gateway database/workspace/session directory and sample file removed after capture. Production state unchanged.
