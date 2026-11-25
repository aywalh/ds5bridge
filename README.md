# DS5 Bridge

![DS5 Bridge for Windows, with USB and Bluetooth support](docs/banner.png)

Use a PS5 DualSense in Windows games that expect an Xbox controller. Plug it in or pair it over Bluetooth, then let DS5 Bridge run quietly in the notification area.

**Windows 10 or 11 · USB or Bluetooth · No account or telemetry**

## From setup to your first game

### 1. Install the bridge

Run `DS5Bridge.exe` and choose **Install**. Open **Options** if you want a desktop shortcut or want the bridge to start with Windows. If the app reports a missing virtual controller driver, install [ViGEmBus](https://github.com/nefarius/ViGEmBus/releases).

<details>
<summary>See the installer</summary>

<p align="center">
  <img src="docs/screenshot-install.png" width="360" alt="DS5 Bridge installer with options below the Install button" />
</p>

</details>

### 2. Connect your DualSense

Use a USB cable, or hold **Create + PS** until the light bar flashes and pair the controller in Windows Bluetooth settings. Open DS5 Bridge from its tray icon. The **Controller** tab shows how the pad is connected and which virtual Xbox slot the game will see.

### 3. Check your inputs and play

Press a button or move a stick. When the preview responds and a virtual slot is listed, launch your game. You can close the window and keep playing. To stop the bridge, choose **Quit** from the tray menu.

![The DS5 Bridge Controller tab with a Bluetooth connection, a virtual Xbox slot and the live preview](docs/screenshot-main.png)

*The screenshot shows demo connection values and the controller drawing included in the distribution build.*

## Make it yours

Start with **Auto** polling. If you want a fixed rate, choose **125, 250 or 500 Hz** on the Controller tab. The **New data** row shows how often fresh reports arrive. If a stick drifts while resting, increase the deadzone slightly.

<details>
<summary>See the appearance and language settings</summary>

Change the accent colour, language and sounds in **Settings**.

![DS5 Bridge settings for updates, sounds, language and accent colour](docs/screenshot-settings.png)

</details>

## Updates and privacy

Controller input and settings stay on your PC. The bridge works offline. An enabled update check reads release information from GitHub. When you choose to install an update, DS5 Bridge compares the downloaded EXE with the release's **SHA-256 checksum** before replacing it.

<details>
<summary>See how an update looks</summary>

![DS5 Bridge showing an example update and its SHA-256 verification notice](docs/screenshot-update.png)

*The version and release notes shown here are examples.*

</details>

## Help and source code

- **Use it without installing:** run `DS5Bridge.exe --portable`.
- **The game sees no controller:** check ViGEmBus and the virtual slot in the Controller tab.
- **The game sees two controllers:** check whether Steam Input or another remapper is reading the same pad.
- **Bluetooth keeps dropping:** remove the controller in Windows Bluetooth settings and pair it again.
- **Uninstall:** go to **Windows Settings → Apps → Installed apps → DS5 Bridge**.

<details>
<summary>Build and verify the package</summary>

```powershell
cargo test --locked --no-default-features
./tools/package.ps1 -Destination ./dist
```

The package contains `DS5Bridge.exe` and its SHA-256 file. The packaging script remaps build paths and scans the EXE for personal paths and email-like strings. The GitHub workflow validates the package when run manually. The repository owner handles publishing.

If the updater cannot access a private release, use the verified EXE and SHA-256 file for a manual update.

</details>

[MIT License](LICENSE) · DS5 Bridge is independent of Sony and Microsoft.
