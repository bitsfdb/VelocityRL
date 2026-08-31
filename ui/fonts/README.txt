Apple Color Emoji (Titles emoji UI)
===================================

Bake method: single Windows TTF + @font-face (not unicode-range chunks).

samuelngs/apple-emoji-ttf ships Linux/Windows TTFs in GitHub Releases; the
web recipe (chunked AppleColorEmoji[n].ttf + CSS) requires building from a
macOS Apple Color Emoji.ttc and is not practical here.

WebView2 limitation (Windows)
-----------------------------
AppleColorEmoji-Windows.ttf is CBDT/CBLC. Chromium WebView2 does **not**
paint CBDT color glyphs loaded via @font-face, so country **flags** stay
blank even when the file loads (preload/@font-face/CSP can all be fine).

Country flags in Titles therefore use Apple Color Emoji PNGs extracted from
the same TTF (not Twemoji):

  ui/flags/{cc}.png   — see ui/flags/README.txt
  ui/main.js          — formatTitleHtml()
  tools/_extract_apple_flags.py — regenerate PNGs from the local TTF

@font-face remains for possible non-flag emoji later; flags must use the
PNG bake-in path. Non-flag emoji may still fall back to Segoe UI Emoji.

Setup (optional TTF for non-flag attempts / re-extract)
-------------------------------------------------------
1. Download AppleColorEmoji-Windows.ttf from:
   https://github.com/samuelngs/apple-emoji-ttf/releases
2. Copy/rename it to:

   tools/fonts/AppleColorEmoji.ttf

   Do NOT put it under ui/ — Tauri embeds frontendDist into the exe (~245MB).

3. (Re)run: python tools/_extract_apple_flags.py

Wiring
------
- ui/style.css   — --emoji-font falls back to Segoe UI Emoji (no bundled TTF)
- ui/flags/      — Apple Color Emoji flag PNGs (WebView2 path that works)
- CSP            — font-src 'self'; img-src 'self' (tauri.conf.json)

Git
---
The TTF is gitignored (~244 MB, Apple copyright). Do not commit or
redistribute it. Flag PNGs under ui/flags/ are fine to keep in-repo (they
are Apple bitmap extracts for offline UI). Do NOT replace
C:\Windows\Fonts\seguiemj.ttf — app-bundled assets only.
