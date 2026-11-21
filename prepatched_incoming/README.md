# Publishing prepatched releases

For each Rocket League build, prepare and test both `TAGame.upk` and
`ProjectX.upk` together. Include all required hooks in these artifacts before
publishing. VelocityRL installs their complete bytes; it never applies local
bytecode, avatar-texture, or palette patches.

Place both files here alongside that build's `RocketLeague.exe`, then run:

```powershell
python scripts/update_prepatched_vps.py --dry-run
python scripts/update_prepatched_vps.py
```

Alternatively provide `--tagame`, `--projectx`, and the exact `--version`
(`261031.63733.533021`, for example). A manually supplied version is an operator
assertion: verify both artifacts were built from that exact release.
The updater never infers a version from a different installed copy of the game
or reuses an old manifest/default version. Partial releases are rejected.
`scripts/server_update_prepatched.py` applies the same policy on the VPS.
Neither script creates the game hooks for you.

Set `VRL_VPS_HOST` and `VRL_VPS_USER` locally. Use an SSH agent or
`VRL_VPS_IDENTITY`; if necessary, set `VRL_VPS_PASS` locally.
Never commit credentials. Dry runs need no credentials.

The existing API serves fixed filenames. Files upload through temporary names;
the manifest is published last. During publication, clients with the previous
manifest may encounter hash mismatches and must retry. Both downloads must pass
size/SHA-256 checks before either live file moves. Uninterrupted publishing would
also require immutable artifact URLs and matching API routing; the API server
implementation is not in this repo.

## Client behavior

| Situation | Behavior |
| --- | --- |
| Exact build, both hashes match | No package writes |
| Exact build, files differ | Stage needed files, verify the complete set, install |
| Different/unknown build | Error; no new packages installed |
| Game running | Defer; close it and retry from Settings |
| API offline/timeout, invalid manifest, bad download | Error; no local patch fallback |
| Missing installed package | Ask user to verify game files in Epic/Steam |
| Locked file, permission failure, full disk | Error; retain originals or roll back |
| Game starts/updates during download | Abort before installation |
| App interrupted during installation | Recover originals on next retry while game is closed |
| Game updated after interrupted installation | Refuse stale recovery; require launcher verification |
| Multiple requests | Serialize through one installer lock |

An unfinished journal lives in `CookedPCConsole/.velocity-prepatched`.
If recovery reports an error, do not launch the game: close file-locking
processes, retry, or use launcher file verification. Replacements are separate
filesystem renames, not a filesystem-wide atomic operation. Keep the game closed
throughout installation. External launcher activity cannot be fully controlled.

## Custom lobby avatars

The earlier working local fix changed `GFxData_LocalPlayer_TA::HandleGetAvatar`
and embedded the user's picture into the `PlaceholderAnthem` texture. A shared
API artifact cannot contain every user's image. To preserve customizable lobby
avatars without client edits, the published package needs a generic local-player
hook that fetches the current image from the local broker.
The client caches and serves the PNG; this change does not build or publish
that new hook. The old static texture callback is not a dynamic lobby hook.
Match/freeplay image broker support remains available.

Restore stock files through Epic/Steam verification. Old `.bak` files can belong
to previous builds, so runtime backup-restoration commands are disabled.
