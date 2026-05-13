# VelocityRL 🚀

**VelocityRL** is the ultimate visual asset swapping tool for Rocket League. Transform your in-game items instantly with a premium desktop experience or a high-performance CLI.

<p align="center">
  <img src="logo.svg" width="128" height="128" alt="VelocityRL Logo">
</p>

## 🌟 Choose Your Version

VelocityRL is maintained in two flavors to suit your workflow:

- **Desktop App (main branch)**: A premium, minimalist Windows application built with Tauri 2.0. Features a beautiful dark UI, sidebar navigation, and a dedicated restoration center.
- **CLI Tool (cli branch)**: A lightweight, interactive terminal wizard for power users who prefer the command line.

---

## ✨ Features

- **Premium SaaS UI**: A stunning dark-mode dashboard with real-time fuzzy search.
- **High-Performance Patching**: Leverages an advanced Python engine for precise `.upk` binary manipulation.
- **Auto-Backups**: Your game files are safe. Backups are created automatically before every modification.
- **One-Click Restoration**: Revert all changes instantly through the dedicated Restore tab or CLI command.
- **Extensive Database**: Fuzzy-search thousands of items from the built-in database.

---

## 🛠️ Installation & Setup

### 🖥️ Desktop App (Recommended)
1. Download the latest installer (`.msi` or `.exe`) from the [Releases](https://github.com/bitsfdb/RLItemMod/releases) page.
2. Run the installer and launch **VelocityRL**.
3. Point the app to your Rocket League data directory in **Settings**.

### ⌨️ CLI Version
```bash
# Install globally
npm install -g velocityrl

# Install engine dependencies
pip install cryptography
```

---

## 🚀 Usage

### Desktop
Simply launch the app, select your **Owned Item**, select your **Target Asset**, and click **Initialize Swap**.

### CLI
Run the interactive wizard from your terminal:
```bash
velocityrl
```

---

## ⚠️ Important Considerations

### Supported Categories
VelocityRL supports most item categories (Decals, Wheels, Boosts, etc.). 

> [!WARNING]
> **Bodies and Goal Explosions** are currently in beta. Swapping these items may cause instability or crashes. Use the **Restore** feature if you encounter issues.

---

## 🤝 Credits & Support

- **Core Engine**: Massive credits to [CrunchyRL/RLUPKTools](https://github.com/CrunchyRL/RLUPKTools) for the Unreal Engine 3 binary patching logic.
- **Developer**: bitsfdb (@sfdb)
- **Discord**: Join our [Support Server](https://discord.gg/2HhBNbrGMj) for updates and help.

---

## 📜 License
MIT
