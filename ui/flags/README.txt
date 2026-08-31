Title flag icons (hosted)
=========================

Flag PNGs are NOT shipped in the app. Host them on the API:

  https://api.velocityrl.tech/thumbnails/flags/flag_{cc}.png

Packaged zip (extract into your thumbnails/flags/ folder):

  tools/flags/thumbnails-flags.zip

Each entry is named flag_xx.png (ISO 3166-1 alpha-2, lowercase).

UI loads them via API_BASE in ui/main.js (flagImgHtml).
Source TTF for regenerating / extracting: tools/fonts/AppleColorEmoji.ttf
