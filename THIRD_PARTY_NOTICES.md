# Third-party notices

SJTU Canvas Downloader's own source code is released under the MIT license in
`LICENSE`. That license does not replace the licenses of the third-party
software built into the desktop apps, listed below.

SJTU Canvas Downloader is an independent, community-built project and is not
affiliated with Shanghai Jiao Tong University, Instructure, or jAccount.
Canvas is a trademark of Instructure, Inc. Other names and marks belong to
their respective owners.

## Acknowledgements

The implementation was informed by these projects:

- [FengYuchen1314/canvas-downloader](https://github.com/FengYuchen1314/canvas-downloader),
  MIT license. Its documented jAccount QR login, Canvas LTI, video-list and
  track-discovery behaviour informed the corresponding clean-room
  implementation in the engine.
- [Neko-Yukari/canvas-sjtu-skill](https://github.com/Neko-Yukari/canvas-sjtu-skill).
  Its public documentation and the observable Canvas REST API behaviour
  informed course and file browsing. That repository did not contain a
  license when reviewed, so no source code was copied.

## Components of the engine

The engine (`sjtu-canvas-engine`) is built from the Rust crates listed in
`engine/Cargo.lock`; each keeps its own license:

- Most crates (Tokio, reqwest, hyper, SQLx, serde, chrono, url,
  chacha20poly1305, tokio-tungstenite and others) are available under the MIT
  and/or Apache 2.0 licenses.
- SQLite is compiled into the engine through `libsqlite3-sys`; SQLite itself
  is in the public domain.
- `scraper` and `ego-tree` (ISC) parse the Canvas and LTI launch pages; they
  use Servo's `selectors`, `cssparser`, `cssparser-macros` and `dtoa-short`,
  which are under the Mozilla Public License 2.0. These MPL files are used
  unmodified; their source is available from crates.io at the versions in
  `engine/Cargo.lock`.
- The ICU4X crates (`icu_*`, `zerovec`, `yoke`, `tinystr`, `writeable`,
  `litemap` and related) are under the Unicode License v3.
- `subtle`, `alloc-no-stdlib` and `alloc-stdlib` are under the BSD 3-Clause
  license; `foldhash` and `zlib-rs` under the zlib license.
- macOS builds use rustls with `rustls-webpki`/`untrusted` (ISC) and
  `webpki-roots`, which carries Mozilla's CA certificate list
  (CDLA-Permissive-2.0). Windows builds use the system TLS (SChannel) through
  `native-tls` and link the Microsoft C runtime statically.

The full list with versions can be produced with
`cargo metadata --manifest-path engine/Cargo.toml --format-version 1`.

## Desktop applications

- Windows: .NET (MIT), Windows App SDK / WinUI 3 (MIT; the redistributed
  runtime binaries are covered by the Windows App SDK license terms) and
  CommunityToolkit.Mvvm (MIT). The app is published self-contained, so these
  runtimes are included in the app folder.
- macOS: the app uses only system frameworks (SwiftUI, AppKit, Security,
  UserNotifications).

## School services

The apps talk to Canvas (`oc.sjtu.edu.cn`), jAccount and the classroom-video
services over their web interfaces, with the user's own login. No
school-provided software is bundled.
