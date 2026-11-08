# Installation

## Requirements

- Windows 10 or 11, or Windows Server 2016 or later, 64-bit (x64).
- Administrator rights. Blocking uses the Windows Filtering Platform, which needs them.
- DirectX 12. If the PC has no usable GPU, such as a VM or a Remote Desktop session, the app falls
  back to the Windows software renderer.

Windows 7, 8.1 and Windows on ARM are not supported.

## Download

Open the [latest release](https://github.com/VARSITYLUNCHBUG/dl-conn-rs/releases/latest). Each
release has these files:

| File | What it is |
| --- | --- |
| `dl-conn-rs.exe` | The app |
| `dl-conn-rs.exe.md5` | MD5 hash of the exe |
| `dl-conn-rs.exe.sha256` | SHA-256 hash of the exe |
| `dl-conn-rs-<version>-source.zip` | Source code for this release |
| `LICENSE.txt` | The MIT licence |
| `THIRD_PARTY_NOTICES.txt` | Licences of the Rust crates built into the exe |

You only need the exe and one hash file to run it.

## Verify the download

Open PowerShell in the download folder and run:

```
(Get-FileHash dl-conn-rs.exe -Algorithm MD5).Hash
Get-Content dl-conn-rs.exe.md5
```

The two values must match, ignoring case. If they don't, delete the exe and download it again.

MD5 catches a damaged or incomplete download. For a stronger check, compare SHA-256 the same way:

```
(Get-FileHash dl-conn-rs.exe -Algorithm SHA256).Hash
Get-Content dl-conn-rs.exe.sha256
```

Without PowerShell, `certutil -hashfile dl-conn-rs.exe MD5` in Command Prompt gives the same value.

## Run

There is no installer. Put `dl-conn-rs.exe` in any folder you like and double-click it. Accept the
admin prompt.

The exe is not code signed, so Windows may say "Unknown publisher", or SmartScreen may stop it
with "Windows protected your PC". After you have checked the hash, choose **More info**, then
**Run anyway**.

Only one copy runs at a time. Starting it again brings the open window to the front.

## Update

Close the app, replace the exe with the new one, and start it again. Settings and the active block
carry over.

## Remove

1. Choose **No block** in the dropdown, or restart the PC later to clear any block.
2. Delete the exe.
3. Delete `%LOCALAPPDATA%\dl-conn-rs`, which holds `settings.ini` and `dl-conn-rs.log`.

dl-conn-rs writes nothing else: no registry keys, no services and no saved firewall rules.

If you run the app as a standard user by entering an administrator's password, the folder is in
that administrator's profile instead.
