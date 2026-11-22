# alertkit

Rule-based alerting for the argus intelligence platform

Extracted from the [argus](https://github.com/WyattAu/argus) intelligence platform
as a standalone, publishable crate.

## Binaries

### `ops-relay`

Tails a line-oriented log and relays every line beyond a persisted offset to a
Telegram chat via [`TelegramSink`](src/telegram.rs), one [`Alert`](src/lib.rs)
per line. The bot token is read from a file at runtime (trimmed) — never pass
tokens on the command line. A missing or empty token file means the relay is
not armed: quiet exit 0, nothing sent. Blank log lines are skipped but still
advance the offset.

```sh
ops-relay --token-file <path> --chat-id <id> --log <path> --offset <path>
```
