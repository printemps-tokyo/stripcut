# Contributing

Thanks for your interest in contributing to stripcut.

## Development

1. Fork / clone the repository
2. Install a stable Rust toolchain (https://rustup.rs)
3. Create a branch: `git switch -c feat/your-change`
4. Make your change and verify locally:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   cargo build --release
   ```
5. Commit and open a pull request

## Adding a platform preset

This is the most common contribution, and it has the strictest rule.

A preset tells somebody their upload will be accepted. That is only defensible
with a citation, so every preset must record the platform page its numbers came
from and the date that page was read. Numbers from a blog post, a forum thread,
a tutorial site or a model's memory are not accepted, however confidently they
are stated. A missing preset is a gap; a wrong preset is a rejected upload after
the artist has already exported thirty files.

Add the entry to `src/presets.rs`. A pull request adding one should:

- set `source_url` to a page the platform itself owns and that states the
  numbers, and `verified_on` to the day you read it
- quote the sentence that establishes each number in the pull request
  description, in the original language
- leave a field `None` when the page does not state it. Do not fill a gap from
  another platform's numbers, and do not round a "recommended" figure into a
  requirement. The tests assert that every preset carries a source and a date
- put a stated width in the right field. `width` is a width the platform
  requires exactly; `min_width` / `max_width` are a permitted range;
  `recommended_width` is advice, which `--check` reports as a note and never as
  a failure. Getting this wrong turns a suggestion into a validation error
- say in `notes` what the source page does not say, not only what it does. That
  text is what a user reads in `--list-presets` when they are deciding whether
  to trust the entry
- add a row to the preset table in `README.md` in the same change
- add a test in `tests/cli.rs` if the preset exercises behaviour the existing
  presets do not

When the whole table is re-verified, bump each entry's `verified_on` in the same
commit as any numbers that moved.

## Detection and planning changes

The cut rule is documented in `README.md` and pinned by tests in `src/plan.rs`
and `tests/cli.rs`. Changing it changes where every existing page gets cut, so a
pull request touching the boundary behaviour needs to say which cases it affects
and add cases on both sides of the boundary.

Two invariants are not negotiable:

- No tile may exceed `--max-height`.
- A cut that severs artwork must be reported as a hard cut, with the tile index
  and the y position. stripcut may cut through a panel when there is no
  alternative; it may never do so quietly.

stripcut deliberately does not attempt panel or region detection, and does not
try to recognise what is in a panel. That is stated plainly in the README rather
than half-solved. Proposals to add computer vision should start as an issue,
because the trade -- a much larger dependency tree, slower runs, and new ways to
be subtly wrong on somebody's artwork -- is a project-level decision.

## Determinism

The same input with the same flags must produce byte-identical tiles and a
byte-identical report. That means: pinned encoder settings, explicit tie-breaks,
no hash-order dependence, and no renderer reading the clock or the terminal
width. There are tests for this; a change that makes output vary run to run will
fail them, and that is the intended outcome.

## Dependencies

stripcut depends on `anyhow`, `clap` and `image`, and that is meant to stay
short. JSON writing, colour parsing, text layout and every rule in the tool are
local code on purpose. A pull request adding a dependency needs to explain why a
few dozen lines of local code is worse. Dependency updates are handled by
Dependabot (see `.github/dependabot.yml`).

## Architecture

Pure logic lives in the library (`src/analyze.rs`, `src/plan.rs`,
`src/encode.rs`, `src/check.rs`, `src/report.rs`) and works on in-memory
buffers. The binary (`src/main.rs`) does argument parsing, filesystem IO and
printing. Keep new logic on the library side: it is the half that can be tested
without a temp directory.

Tests build their canvases programmatically. Please do not add binary fixtures.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/) are preferred
(`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`). English, no emoji.

## Pull requests

- Keep one PR focused on a single purpose
- CI (fmt / clippy / test / build) must be green
- Update the README and tests when behavior changes

## Bug reports and requests

Please open an [issue](../../issues) using the templates. For a page that cut in
the wrong place, the most useful thing you can include is the output of
`stripcut yourpage.png --gutters` and the flags you used -- that usually shows
immediately whether detection or planning is at fault.
