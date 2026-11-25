Mandrake @VERSION@ — Myth II: Soulblighter & Myth: The Fallen Lords Source Port
===============================================================================

Game data is NOT included. You need your own legal installation of:
  - Myth II: Soulblighter patched to 1.8.5 (https://projectmagma.net/)
  - Myth: The Fallen Lords patched to 1.5 (optional, for --game tfl)

Quick Start
-----------
1. Extract this archive directly into your Myth II folder (the folder holding
   tags/, plugins/, recordings/, and so on):

     Myth II/
       mandrake (or mandrake.exe / Mandrake.app)
       mandrake.original.toml
       tags/
       plugins/
       recordings/

2. Double-click mandrake.exe (Windows), Mandrake.app (macOS), or ./mandrake.sh
   (Linux / Steam Deck).

   If you run Mandrake from another folder, it will prompt you to select your
   Myth II folder and save the location into mandrake.toml automatically.

First-Launch Warnings (Unsigned Builds)
---------------------------------------
- Windows SmartScreen: click "More info", then "Run anyway".
- macOS Gatekeeper: right-click Mandrake.app -> Open (or open System Settings
  -> Privacy & Security -> Open Anyway, or run:
      xattr -dr com.apple.quarantine Mandrake.app
  in Terminal).

Portable Settings & Cache
-------------------------
Mandrake is fully portable:
- On first run, mandrake.toml is created automatically beside mandrake using
  the Modern-Retro defaults (smooth visuals, Lanczos-2 UI, TFL smoothing).
- Caches (such as TFL sprite interpolation) are stored in cache/ beside
  mandrake.
- To use the classic 1.8.5 / TFL 1.5 look instead, launch with:
      --conf mandrake.original.toml

Playing Myth: The Fallen Lords (TFL 1.5)
----------------------------------------
Create a shortcut to mandrake with the launch option:
    --game tfl

If your TFL folder is not beside mandrake, Mandrake will prompt you to pick it
(or set [game] tfl_data = "/path/to/Myth_TFL" in mandrake.toml). On first TFL
launch, Mandrake downloads the MIT-licensed RIFE v4.6 weights once and builds
the sprite interpolation cache automatically.

Steam Deck (Gaming Mode)
------------------------
1. In Desktop Mode, extract this archive into your Myth II folder (or unpack to
   ~/Games/mandrake and place your Myth II folder at ~/Games/mandrake/data).
2. In Steam (Desktop Mode): Games -> Add a Non-Steam Game -> Browse -> select
   mandrake.sh (set file filter to "All Files").
3. Properties -> Compatibility: leave unchecked or choose "Steam Linux Runtime".
   Never force Proton (native Linux binaries will crash under Proton).
4. Optional launch options: --game tfl  or  --conf mandrake.original.toml
5. In Gaming Mode, use a mouse+keyboard Steam Input layout (trackpad as mouse).

Licences
--------
Mandrake's source code is dual-licensed under MIT OR Apache-2.0 (LICENSE-MIT,
LICENSE-APACHE). Third-party licences are in THIRD_PARTY_NOTICES.md.
Myth II and Myth: The Fallen Lords game data remain the property of their
respective owners.
