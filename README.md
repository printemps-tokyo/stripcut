# stripcut

Slice a long vertical-scroll comic canvas into upload-ready tiles, cutting in
the gutters between panels instead of through the artwork. Offline Rust CLI.

A webtoon episode is drawn as one continuous canvas -- 800 pixels wide and tens
of thousands of pixels tall -- but every platform wants it uploaded as a
sequence of images, each under a maximum height and a maximum file size. So the
canvas has to be cut into thirty-odd pieces.

Cutting every 1280 pixels is trivial and wrong. A fixed interval lands wherever
it lands, which sooner or later is halfway down a face, through a speech balloon
or across the punchline panel. stripcut looks for the gutters -- the runs of
rows that hold nothing but page background -- and cuts there, as close to the
tile height you asked for as the artwork allows.

```
$ stripcut episode_07.png --preset webtoon-canvas
stripcut: slice
canvas: 800x11160 px from 1 file
preset: webtoon-canvas (WEBTOON CANVAS), verified on 2026-08-12
settings: max-height 1280, min-height 320, target-height 1280 (= max-height)
          min-gutter 12, tolerance 6, edge-margin 0, background #ffffffff (auto)
          format png, quality 85, max-bytes unlimited
out-dir: out

    #  file                            y-range         height  cut            bytes
    1  episode_07_001.png                      0-805     805  gutter         4452   4.3 KiB
    2  episode_07_002.png                   805-2060    1255  gutter         6817   6.7 KiB
    3  episode_07_003.png                  2060-2700     640  gutter         3540   3.5 KiB
    4  episode_07_004.png                  2700-3980    1280  HARD-CUT       6927   6.8 KiB
    5  episode_07_005.png                  3980-5175    1195  gutter         6509   6.4 KiB
   ...

11 tiles, 3 hard cuts, 59.5 KiB total, 11 written

warnings:
  ! tile 4: no gutter between y=3020 and y=3980; cut forced at y=3980 (inspect this seam)
```

Everything runs locally. No network access, no image is ever uploaded anywhere,
and there is no CV or ML dependency -- gutter detection is pixel arithmetic.

## Install

```bash
git clone https://github.com/printemps-tokyo/stripcut
cd stripcut
cargo build --release
# the binary is at ./target/release/stripcut
```

Or grab a prebuilt tarball from the
[releases page](https://github.com/printemps-tokyo/stripcut/releases); every
artifact ships with a `.sha256` next to it.

## Quickstart

```bash
# See what stripcut can find before you commit to anything
stripcut episode.png --gutters

# Plan the cuts without writing a byte
stripcut episode.png --dry-run --max-height 1280

# Slice for real, into ./out
stripcut episode.png --preset webtoon-canvas -o out

# Several parts of one episode, stacked and sliced as one canvas
stripcut part_a.png part_b.png part_c.png --join --preset webtoon-canvas

# Validate what you are about to upload
stripcut out --check --preset webtoon-canvas
```

## How gutter detection works

Four steps, each with a knob attached.

1. Find the page background. stripcut samples the edge columns and the top and
   bottom rows and takes the most common colour. A fully transparent pixel
   always counts as background, whatever its colour channels hold. Override the
   guess with `--bg ffffff`, or say the page is transparent with `--bg 0000`.

2. Decide which rows are empty. A row is a gutter row when every pixel in it is
   the background colour, within `--tolerance` per channel. The default
   tolerance is 6, which absorbs JPEG ringing and a soft paper texture without
   letting real artwork through. `--edge-margin N` ignores N pixels at each side
   of every row, which is how you get past a printed border or a watermark
   stripe running down the page.

3. Collect the runs. Consecutive gutter rows form a run; runs shorter than
   `--min-gutter` (default 12) are discarded, so a one-pixel seam between two
   touching panels is not mistaken for a place to cut.

4. Choose the cuts. Standing at the last cut, the next one must land between
   `--min-height` and `--max-height` below it. Inside that window stripcut takes
   the gutter whose centre is closest to `--target-height` (or `--max-height`
   when you do not set one) and cuts at that centre, so both tiles keep an equal
   margin of background. Ties go to the higher gutter, which makes the plan a
   deterministic function of its inputs.

A white speech balloon inside a panel is not a gutter: its rows contain artwork
somewhere along their width, so they fail step 2. That is the whole reason
detection is row-based rather than region-based.

When a page is not cutting where you expect, run `--gutters` first. It prints
exactly what stripcut can see:

```
    #  start        end     height     centre
    1      760        850         90        805
    2     2030       2090         60       2060
    3     2630       2770        140       2700

9 gutter runs at --min-gutter 12, covering 860 rows; tallest is 160 px
```

## The honest limitation

Detection is pixel-based. It assumes the page has a mostly uniform background,
which is how vertical-scroll comics are normally laid out -- but that assumption
does not always hold.

A full-bleed illustration that runs edge to edge for 4000 pixels has no empty
rows anywhere. There is no gutter to find, so stripcut cuts at `--max-height`
and severs artwork. It cannot avoid that, and it does not hide it: the tile is
labelled `HARD-CUT` in the report, `"cut": "hard-cut"` in the JSON, and a warning
names the tile and the exact y position so you can go and look at that seam.
`totals.hard_cuts` counts them.

A hard cut is a warning, not a failure. The exit code stays 0, because a
full-bleed page is a legitimate thing to slice -- you just want to know where
the scars are. If your workflow should fail on them, check
`totals.hard_cuts` in the `--json` output.

Two things that help before you give up on a page:

- Raise `--tolerance` if the background is a scan or a JPEG and its white is not
  exactly white.
- Lower `--min-gutter` if the layout is tight and the gaps are only a few pixels.

## Platform presets

Every preset carries the platform-owned page its numbers came from and the date
that page was read. A number that could not be verified against an official
creator-facing page is not in the table -- the field is `null` and the
corresponding flag has to supply it. stripcut is fully usable with no preset at
all: `--max-height`, `--min-height`, `--target-height` and `--max-bytes` cover
every case a preset would.

| preset | platform | width | height | bytes | images | formats | source (verified 2026-08-12) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `webtoon-canvas` | WEBTOON CANVAS | max 800 | max 1280 | - | - | - | [notice 1766](https://www.webtoons.com/en/notice/detail?noticeNo=1766) |
| `line-manga-indies` | LINE マンガ インディーズ (vertical) | 400-10000, 800 recommended | 600-10000 | 2,000,000 per image | max 1000 | png, jpg | [guideline](https://manga.line.me/indies/guideline) |
| `comico` | comico (manga, vertical) | 800 recommended | max 10000 | 2,000,000 per image, 20,000,000 per episode | max 100 | jpg, png | [manga guideline](https://resources.comico.io/html/legal/manga_guideline.html) |
| `tapas` | Tapas | 940 required | - | 10,000,000 per image | - | png, jpg, gif | [file size overview](https://help.tapas.io/hc/en-us/articles/360052100813-File-Size-Overview) |
| `mechacomic-creators` | めちゃコミック クリエイターズ (vertical) | 300 recommended | - | 8,000,000 per image, 150,000,000 per episode | max 60 | jpg, png, gif | [submission manual](https://creators.mechacomic.jp/manual) |

`stripcut --list-presets` prints the same table with each preset's full notes,
including what its source page does not say. `--list-presets --json` emits it as
a machine-readable document.

Notes on the entries:

- WEBTOON's notice states 800x1280 as the maximum image dimensions the uploader
  auto-slices down to, not as a required canvas size, so 800 is recorded as a
  maximum width rather than a required one.
- Widths are three separate fields, because platforms mean three different
  things by "width": required exactly, permitted within a range, and merely
  recommended. A recommendation is reported by `--check` as a note and never as
  a failure. Collapsing the three would turn advice into a validation error.
- `tapas` and `mechacomic-creators` publish no maximum image height, so
  `--max-height` is required when you use them. `--list-presets` says so per
  preset under "plans alone".
- Tapas's help article blocks non-browser clients; open the URL in a browser to
  re-read it.
- Piccoma has no preset. No official page stating concrete image limits could be
  found, and the numbers circulating for it online are LINE Manga's numbers
  echoed back. A missing preset is a gap; a wrong preset is a rejected upload.

Platforms change their guidelines. Confirm against the current page before you
upload; stripcut prints that caveat under `--list-presets` and at the foot of
every report that used a preset.

## Modes

| mode | what it does | writes |
| --- | --- | --- |
| default | plans the cuts and writes the tiles | yes |
| `--dry-run` | prints the planned cuts with estimated sizes | no |
| `--gutters` | prints the detected runs of background rows and stops | no |
| `--check` | validates already-sliced images against the limits and stops | no |

`--check` is the pre-upload gate. Point it at the directory you are about to
upload and it reports, per file, whether the platform will take it -- width,
height, file size, format, and the episode-level image count and total size
where the platform publishes them. It exits non-zero on any violation, so it
drops straight into a script.

```
$ stripcut out --check --preset webtoon-canvas
stripcut: check
preset: webtoon-canvas (WEBTOON CANVAS), verified on 2026-08-12

limits: width <= 800 px, height <= 1280 px

  OK    out/episode_07_001.png    800x805           4452  png
  FAIL  out/episode_07_002.png    800x4000         12908  png
          - height is 4000 px, maximum is 1280 px

2 images checked, 17.0 KiB total, 1 failed
```

## Flags

Inputs are files, or directories whose `png` / `jpg` / `jpeg` / `webp` files are
taken in sorted order (stripcut does not recurse). Files named on the command
line keep the order you typed them, which is what `--join` needs.

| flag | default | meaning |
| --- | --- | --- |
| `-o`, `--out-dir DIR` | `./out` | where the tiles are written |
| `--preset NAME` | none | take limits from a platform preset |
| `--list-presets` | | print the preset table and exit |
| `--max-height PX` | from the preset | hard ceiling on a tile's height, never exceeded |
| `--min-height PX` | a quarter of `--max-height` | floor on a tile's height |
| `--target-height PX` | `--max-height` | the tile height to aim for |
| `--max-bytes BYTES` | from the preset | per-tile byte budget |
| `--min-gutter PX` | 12 | shortest run of background rows that counts as a gutter |
| `--tolerance N` | 6 | per-channel slack when deciding a pixel is background (0-255) |
| `--edge-margin PX` | 0 | ignore this many pixels at each side of every row |
| `--bg HEX` | auto-detected | page background, e.g. `ffffff` or `0000` for transparent |
| `--join` | off | treat every input as one continuous canvas |
| `--format FMT` | the source format | `png`, `jpg` or `webp` |
| `--quality N` | 85 | encoder quality for `jpg`, 1-100 |
| `--prefix NAME` | the input's file stem | output filename prefix |
| `--start-index N` | 1 | number the first tile from here |
| `--dry-run` | off | print the planned cuts, write nothing |
| `--check` | off | validate already-sliced images and stop |
| `--gutters` | off | print the detected gutter runs and stop |
| `--json` | off | emit the machine-readable document |
| `--force` | off | overwrite existing output files |
| `--no-color` | off | never emit ANSI colour codes |
| `-q`, `--quiet` | off | print only the one-line summary |

Output filenames are `{prefix}_{number}.{ext}`, zero-padded to at least three
digits and wider when the episode needs it, so the sequence sorts correctly in
any file browser or shell glob. Numbering runs on across a directory of inputs
rather than restarting at 1 for each file.

Notes on a few of them:

- `--max-bytes` is a per-tile ceiling. For `jpg`, stripcut encodes at
  `--quality` and, if the tile is over budget, walks a fixed quality ladder
  (85, 75, 65, 55, 45, 38, 30) and takes the first step that fits. It never goes
  below 30: a tile that still does not fit there needs a shorter tile, not a
  worse one.
- PNG and WebP are lossless in stripcut, so `--quality` does not apply to them
  and the ladder cannot help. (The `image` crate's WebP encoder is VP8L
  lossless-only.) A lossless tile over budget is reported as an `oversize`
  warning that says exactly that, and the run exits 1. It is never silently
  passed off as compliant.
- Existing output files are never overwritten without `--force`. Every target
  path is checked before any byte is written, so a run either writes the whole
  episode or refuses up front, leaving what was already there untouched.
- Colour is off automatically when stdout is not a terminal, when `NO_COLOR` is
  set, and always under `--json`.

## Determinism

The same input with the same flags produces byte-identical tiles and a
byte-identical report. Encoder settings are pinned rather than taken from the
crate defaults, tie-breaks in cut selection are explicit, background detection
breaks ties by colour value rather than hash order, and no renderer reads the
clock or the terminal width. This is covered by tests.

## Exit codes

| code | meaning |
| --- | --- |
| 0 | the run did what it was asked to do |
| 1 | a limit was breached: a `--check` violation, or a tile that cannot be brought under `--max-bytes` |
| 2 | usage error, unreadable input, or a refusal to overwrite without `--force` |

A hard cut does not affect the exit code. See "The honest limitation" above.

## JSON

`--json` always emits the same envelope, whether one canvas was processed or
twenty. A consumer never has to branch on how many inputs were given.

```jsonc
{
  "tool": "stripcut",
  "version": "0.1.0",
  "schema": 1,
  "ok": true,              // false if any run breached a limit
  "runs": [
    {
      "mode": "slice",     // "slice" | "dry-run" | "gutters" | "check"
      "ok": true,
      "inputs": ["episode.png"],
      "out_dir": "out",    // null unless tiles were written
      "preset": {          // null when no --preset was given
        "name": "webtoon-canvas",
        "platform": "WEBTOON CANVAS",
        "width": null, "min_width": null, "max_width": 800,
        "recommended_width": null,
        "min_height": null, "max_height": 1280,
        "max_bytes": null, "max_total_bytes": null, "max_tiles": null,
        "formats": [],
        "can_plan": true,
        "notes": "...",
        "source_url": "https://...",
        "verified_on": "2026-08-12"
      },
      "canvas": {
        "width": 800,
        "height": 11160,
        "sources": [{"path": "episode.png", "width": 800, "height": 11160}]
      },
      "settings": {
        "max_height": 1280,        // null in --gutters mode
        "min_height": 320,         // null in --gutters mode
        "target_height": null,
        "min_gutter": 12,
        "tolerance": 6,
        "edge_margin": 0,
        "background": "#ffffffff",
        "background_source": "auto",  // "auto" | "flag"
        "format": "png",           // null in --gutters mode
        "quality": 85,             // null in --gutters mode
        "max_bytes": null
      },
      "gutters": [
        {"start": 760, "end": 850, "height": 90, "centre": 805}
      ],
      "tiles": [
        {
          "number": 1,             // the number in the filename
          "file": "episode_001.png",
          "y_start": 0,
          "y_end": 805,
          "height": 805,
          "cut": "gutter",         // "gutter" | "hard-cut" | "end"
          "gutter": {"start": 760, "end": 850, "height": 90, "centre": 805},
          "bytes": 4452,
          "quality": null,         // the quality settled on, lossy formats only
          "written": true
        }
      ],
      "checked": [                 // --check only
        {
          "path": "out/t1.png",
          "width": 800, "height": 805, "bytes": 4452,
          "format": "png",
          "ok": true,
          "problems": [],          // one sentence per violation
          "notes": []              // advisory, never affects the exit code
        }
      ],
      "limits": null,              // --check only: what was enforced
      "episode_problems": [],      // --check only: too many images, too many bytes
      "warnings": [
        {
          "kind": "hard_cut",      // hard_cut | no_gutters | short_tile | oversize | input
          "tile": 3,               // 0-based plan index, or null
          "message": "no gutter between y=3020 and y=3980; cut forced at y=3980 ..."
        }
      ],
      "totals": {
        "tiles": 11, "written": 11, "bytes": 60930, "hard_cuts": 3,
        "gutters": 9, "warnings": 3, "checked": 0, "failed": 0
      }
    }
  ],
  "caveat": "Preset limits were read from the official pages ..."
}
```

`schema` is bumped whenever a field changes meaning. `--list-presets --json`
emits a separate document with the same `tool` / `version` / `schema` /
`caveat` framing and a `presets` array.

## Limits

stripcut refuses a canvas wider than 20,000 px, taller than 200,000 px, or over
120 megapixels in total, before allocating for it. An RGBA buffer is 4 bytes per
pixel, so 120 MP is roughly 480 MB resident. A joined canvas is held to the same
cap.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --release
```

The pure logic -- background detection, row emptiness, gutter runs, cut
planning, encoding, the `--check` rules, rendering -- lives in the library and
works on in-memory buffers, so it is unit-tested without touching the
filesystem. The binary does argument parsing, IO and encoding, and nothing else.
The test suite builds every canvas programmatically; there are no fixtures and
nothing reaches the network.

See [CONTRIBUTING.md](CONTRIBUTING.md). The rule for presets is strict: no
number without an official page and the date you read it.

## License

MIT. See [LICENSE](LICENSE).
