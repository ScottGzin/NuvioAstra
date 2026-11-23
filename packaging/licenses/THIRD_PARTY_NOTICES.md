# Nuvio Astra — third-party notices

Modifications and new code: Copyright (c) 2026 ScottGzin and Nuvio Astra contributors.
Original code: Copyright of the Nuvio team / NuvioMedia and contributors.

Nuvio Astra is free software under the **GNU General Public License v3.0**
(`NUVIO-ASTRA-LICENSE-GPL-3.0.txt`). It is a fork of **Nuvio Desktop**
(https://github.com/NuvioMedia/NuvioDesktop, GPL-3.0). Its complete source code is published in the
same GitHub repository as the installers (tag of each release).

The Windows installer ships these separately built components:

## Nuvio Engine — `nuvio_engine.dll`

- Project: https://github.com/NuvioMedia/nuvio-engine (GPL-3.0-or-later), release **v0.1.6**,
  asset `nuvio-engine-windows-x86_64-0.1.6.zip`, unmodified.
- DLL SHA-256: `84a149c16bea5ac376a7468c61c7bb2d5f3d8fc8e1b74a7980af2edd8f426743`.
- Contains libtorrent 2.0.12 (BSD-3-Clause), Boost 1.86.0 headers (BSL-1.0), OpenSSL 3.5.7
  (Apache-2.0), try_signal (BSD-3-Clause, Copyright (c) 2016 Arvid Norberg), built with llvm-mingw (Apache-2.0 with LLVM exception).
- Its license texts, notices and build information are in `nuvio-engine/`, as published by the
  project. Corresponding source: the project's repository at tag `v0.1.6`.

## libmpv — `libmpv-2.dll`

- mpv **v0.40.0-465-gf6c116491** (https://github.com/mpv-player/mpv, commit `f6c116491`), the same
  Windows x64 build Nuvio Desktop ships, unmodified.
- mpv is GPL-2.0-or-later (LGPL-2.1-or-later when built without GPL parts); this Windows build links
  GPL components (FFmpeg built with GPL options, libass, libplacebo, LuaJIT, and others), so it is
  distributed under the **GPL**, compatible with Nuvio Astra's GPL-3.0.
- Sources: mpv — https://github.com/mpv-player/mpv ; FFmpeg — https://ffmpeg.org/download.html ;
  Windows build recipe used by these builds — https://github.com/shinchiro/mpv-winbuild-cmake .
  On request, the maintainers of this repository provide the corresponding source of the exact
  build as required by the GPL.

## Anime4K (shaders compiled into `Nuvio Astra.exe`)

- Project: https://github.com/bloc97/Anime4K (MIT, Copyright (c) 2019 bloc97), commit
  `7684e9586f8dcc738af08a1cdceb024cc184f426`: `Clamp_Highlights`, `Restore_CNN_{S,M,VL}`,
  `Restore_CNN_Soft_{S,M,VL}`, `Upscale_CNN_x2_{S,M,VL}`, `Upscale_Denoise_CNN_x2_{M,VL}`,
  `Denoise_Bilateral_Mode`, `Deblur_DoG`, `Thin_HQ`, `Darken_HQ`. The files are unmodified; at
  playback only the strength `#define` of the last four changes with the level the user chose.
- `AutoDownscalePre_x2` and `AutoDownscalePre_x4`, from the same project, are public domain
  (Unlicense, https://unlicense.org; the text is at the top of each file).
- License text: `anime4k/ANIME4K-LICENSE.txt`.

## Rust crates and the web interface (compiled into `Nuvio Astra.exe`)

- Tauri 2 and its plugins (`tauri`, `tauri-plugin-updater`): MIT or Apache-2.0.
- React, React DOM: MIT. Vite and TypeScript are build tools only (MIT / Apache-2.0).
- Other Rust crates (tokio, reqwest, serde, libloading, windows, webview2-com, minisign-verify and
  their dependencies): MIT and/or Apache-2.0 (some BSD-3-Clause, ISC, Zlib, Unicode-3.0). The full
  list with licenses can be produced with `cargo license` or `cargo about` from the source tree.

## License texts: React and Tauri

React and React DOM:

```text
MIT License

Copyright (c) Meta Platforms, Inc. and affiliates.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Tauri and its plugins (used under the MIT option):

```text
MIT License

Copyright (c) 2017 - Present Tauri Apps Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Not shipped

- Microsoft Edge WebView2 Runtime: part of Windows 10/11 (or installed by Microsoft), not
  redistributed by this installer.
- No codec, font or media file is included.
