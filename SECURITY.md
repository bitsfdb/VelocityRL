# Security Policy

## Supported Versions

We actively provide security patches for the following versions of VelocityRL:

| Version | Supported |
| --- | --- |
| `2.1.x` (Latest) | :white_check_mark: |
| `2.0.x` | :white_check_mark: |
| `< 2.0.0` | :x: |

---

## Reporting a Vulnerability

If you find a security issue in VelocityRL, please disclose it responsibly. Do not use public GitHub issues or public Discord channels to report security flaws.

### Primary method: GitHub Private Vulnerability Reporting (Preferred)
Submit a report directly via the repository security tab:
* Navigate to **Security** > **Advisories** > **Report a vulnerability**.
* This creates an encrypted, private thread directly between you and the maintainers.

### Alternative method: Direct Email
Send an email to **[bits@bndq.me](mailto:bits@bndq.me)**:
* Subject: `[SECURITY] VelocityRL Vulnerability Report`
* For sensitive proof-of-concept exploits, please encrypt your report using our PGP key (available via key servers or linked on profile).

> *Note regarding Discord:* You may ping `@sfdb` on our [Discord server](https://discord.gg/2HhBNbrGMj) solely to notify us that an email or advisory was submitted. Never paste raw PoCs, reproduction steps, or payloads into Discord tickets.

---

## What to Include in Your Report

To help us verify and patch the vulnerability quickly, include:
* **Summary & Impact**: Description of the vulnerability and what an attacker can achieve.
* **Reproduction**: Clear step-by-step instructions or a minimal Proof of Concept (PoC).
* **Target Details**: VelocityRL version, operating system, and architecture.
* **Artifacts**: Relevant crash dumps, console logs, or sample payloads (e.g., malformed presets or map archives).

---

## Response & Triage Timeline

* **Acknowledgment**: Within **48 hours** of receiving your report.
* **Triage & Assessment**: Confirmation of reproducibility and severity within **5 business days**.
* **Remediation**: We aim to release a patched build within **14 days** depending on complexity.
* **Coordinated Disclosure**: We follow coordinated disclosure. We ask that reporters wait until a patch is published (or up to 90 days) before publicly sharing details.
* **Credit**: We will credit your findings in our release notes and GitHub security advisory (unless you request anonymity).

---

## Scope

### In Scope
* **Tauri Desktop Application**: Vulnerabilities in Rust IPC commands, Tauri plugins, or webview sandboxing that could yield Remote Code Execution (RCE), Local Privilege Escalation (LPE), or Arbitrary File Writes.
* **Local Proxy & Interceptor Engine**: Insecure local TLS certificate generation/handling, path traversal, or network listeners exposing sensitive sockets to the local network (`proxy.rs`, `psynet.rs`).
* **Archive Extractors & Parsers**: Path traversal (Zip Slip), out-of-bounds writes, or parser crashes during workshop extraction or preset deserialization (`workshop.rs`, `presets.rs`, `upk/`).
* **Integrity System**: Flaws permitting unauthorized binary tampering bypasses or untrusted arbitrary code loading through asset hooks (`integrity.rs`, `avatar_manager.rs`).

### Out of Scope
* First-party servers or infrastructure outside of our control (Epic Games, Psyonix, Steam).
* Attacks requiring physical access to an unlocked, pre-compromised operating system.
* Intentional local memory modifications or cosmetic asset replacements that do not cross system privilege boundaries.
* Denial of service (DoS) attacks targeting local client process stability without privilege escalation.

---

## Safe Harbor

Any research performed in accordance with this policy is considered authorized. If you make a good-faith effort to avoid privacy violations, data destruction, and service degradation, we will not initiate legal action against you or request law enforcement involvement.
