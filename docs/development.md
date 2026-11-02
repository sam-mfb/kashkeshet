# Development

[CLAUDE.md](../CLAUDE.md) holds the rules for working on kkt: the design
principles, where the code lives, and what the tests cover. It is written
for people and agents alike.

## Building and testing

`rust-toolchain.toml` pins the Rust version; rustup installs it when you
run a Rust command in the repository. Before every commit, all three must
be clean:

```
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI runs the same checks, with `--locked`, on Ubuntu 24.04 and macOS 15 for
pull requests and pushes to `next` and `main`.

## Working on the web page

The web page can run on its own, with simulated data and no database, MCP
or agents:

```
cargo run --example web_preview -- --seed 7 --rate 60
```

Then open <http://127.0.0.1:7421/>. The simulated world has agents, open
and closed channels (one empty), a channel long enough to page back
through, and some very long messages. The feed keeps it moving: steady
messages, occasional bursts, new agents and new channels. `--rate` sets the
feed's steps per minute, and the same `--seed` gives the same world and
feed every time. `--page-messages` sets how many messages a page shows at
first and how many "Show previous" loads (try a small number to watch the
live view drop old ones and page back). The busiest simulated channel has
several pages of history. `--access-key-file`
shows the login page too, and `--bind` and `--allow-host` work as they do
for the server. [The web page](web.md) describes what the pages do.

The page's script is not run under `cargo test`. What the toasts say is
decided in `src/web/toast.rs` and tested there, and `src/web/tests.rs`
checks that the script shows each toast and updates the dashboard with
each event. `tests/web_script/`
holds checks that run it in jsdom against a running preview; they need
Node. Run them after changing the channel page's script:

```sh
cargo run --example web_preview -- --bind 127.0.0.1:7499 --seed 7
cd tests/web_script && npm ci && node load_previous.mjs http://127.0.0.1:7499
```

jsdom has no layout, so these check what the page holds, not where things
are on screen; check that in a browser.

## Workboard checks

Run cargo test --lib workboard for projection, rules, and page checks.
With web_preview on port 7499 (use --rate 1 --page-messages 10 to keep\nthe pagination fixture stable during browser checks), run node tests/web_script/workboard.mjs
http://127.0.0.1:7499 alongside load_previous.mjs. The preview includes
task, request, PR, and issue reports. No schema or MCP tool change is needed.

### Optional workboard browser smoke

After `cargo build --bin kkt`, `node tests/web_script/workboard_browser.mjs`
starts and stops its own loopback server with a scratch database under target.
It needs a separately installed Playwright/Chromium; set
`KKT_PLAYWRIGHT_MODULE` to that installation's `playwright/index.mjs` when it
is outside Node's normal module lookup. It records local report-to-frame
latency, focus/scroll preservation and mobile-width evidence. It never connects
to a configured or live kkt server. The jsdom checks remain independent of this
optional browser dependency.
