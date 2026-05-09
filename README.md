# RLItemMod

A powerful, user-friendly Node.js CLI tool for performing visual asset swaps in Rocket League.

## Overview

`RLItemMod` provides an interactive terminal wizard that allows you to swap in-game items (e.g., swapping a standard boost for Alpha Reward). Under the hood, it seamlessly invokes the advanced `RLUPKTools` Python engine to accurately parse `.upk` encryption, perfectly expand Name Table string offsets, and rebuild the package architecture without causing game crashes.

## Features

- **Interactive Wizard**: A beautiful command-line interface to search for and select your source and target items.
- **Python Interop**: Leverages a robust Python backend to handle complex LZO decompression, AES decryption, and binary offset shifting.
- **Automated Backups**: Automatically backs up original game assets before patching, with a one-click CLI restore feature.
- **Item Database**: Uses a built-in `items.json` database for fuzzy-searching and mapping in-game item names directly to their underlying UPK files.

## Installation

### Prerequisites

- Node.js (v18+)
- Python 3.8+ (must be available in your system PATH)

### Global Install (Recommended)

```bash
npm install -g rl-item-mod
```

### Local Development

```bash
git clone https://github.com/bitsfdb/RLItemMod.git
cd RLItemMod
npm install
npm run build
npm link
```

## Usage

Simply launch the interactive wizard from your terminal:

```bash
rl-item-mod
```

Or run directly via npx:

```bash
npx rl-item-mod@latest
```

## Credits

Massive credits to [CrunchyRL/RLUPKTools](https://github.com/CrunchyRL/RLUPKTools) for making this repository possible. The advanced Python engineering for parsing and shifting Unreal Engine 3 UPK binaries was instrumental in making this project work safely.

## License

MIT
