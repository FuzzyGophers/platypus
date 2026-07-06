# SDS150 — Display customization reference

Spec-ready reference for the SDS150's on-screen **display customization**, from the Uniden
File Specification V2.00 (the `Display Customize` records + the `DiplsyCustomSDS100` and
`Color, Item code` sheets). This is what a Platypus "theme your scanner display" feature
writes. Supplements [`sds150.md`](sds150.md); see the [TODO](../../TODO.md) item.

**Where it lives:** the scanner's **settings/config file** — *not* the HPDB browse database
or the favorites lists (the two file classes Platypus writes today). So this is a third file
class. It's still bulk SD-card programming, so it rides the same rules: `fsync` + eject, and
the **byte-exact round-trip is the writer safety gate**. Delete `app_data.cfg` after writing.

## The records

Four records drive the display. Formats (fields after the command):

- **`DisplayOption`**: global display settings.
  - `MotTgidFormat` `DEC`/`HEX` · `EdacTgidFormat` `AFS`/`DEC` · `Simple mode` `Off`/`On` ·
    `Color Mode` `COLOR`/`BLACK`/`WHITE` · `ScnDisp Mode` `Mode 1`/`Mode 2` · `Upside_down`
    (no effect on SDS100). Other columns are `Reserve`.
- **`Backlight`**: `FlashLed` `Off`/`On` · `SQ Light` `Off`/`5`/`10`/`15`/`OpenSquelch` ·
  `Key Light` `15`/`30`/`60`/`120`/`Infinite`. If `FlashLed`=On, the Alert LED flashes when the
  backlight turns off. (The V2.00 spec marks `Brightness`, `Key_Backlight`, `Ext_PWR_Light`,
  `Dimmer_*` as SDS100/200-only, but a **real SDS150 card populates `Brightness` (`High`) and the
  dimmer fields** too — so map columns from the card and preserve the ones you don't edit.)
- **`DispOptItems`**: *which data items appear*, per option-area group and layout mode.
  Format: `DispOptItems DispOptId DispLayoutId OptItem1 OptItem2 … OptItemN`. Item order is
  fixed by `(DispOptId, DispLayoutId)`; each `OptItemK` is a **File token** from the item
  tables below.
- **`DispColors`**: *per-item text + background colors*, per color group and layout mode.
  Format: `DispColors DispColorId ColorLayoutId TextColor1 BackColor1 TextColor2 BackColor2 …`.
  Each color is a 6-hex value from the palette below. `F` and `Soft Key 1/2/3` use **reversed**
  text/back colors.

Example (real card, Simple Conventional):
```
DispOptItems DispOptId=1 DispLayoutId=1 FL_Name Empty Frequency
DispOptItems DispOptId=2 DispLayoutId=1 ServiceType CTCSS/DCS
DispOptItems DispOptId=3 DispLayoutId=1 ATT Empty Day
DispOptItems DispOptId=4 DispLayoutId=1 Modulation P_Ch IFX
DispColors    DispColorId=1 ColorLayoutId=1 ff4600 000000
DispColors    DispColorId=3 ColorLayoutId=1 ffffff 000000
DispColors    DispColorId=4 ColorLayoutId=1 e79473 000000
```

## Layout modes (`DispLayoutId` / `ColorLayoutId`)

The two ids select the display mode; note they differ between the option and color records.

| Mode | `DispLayoutId` | `ColorLayoutId` |
|---|---|---|
| Simple Conventional | 1 | 1 |
| Simple Trunk | 2 | 6 |
| Detail Conventional | 3 | 2 |
| Detail Trunk | 4 | 7 |
| Search / Close Call | 5 | 3 |
| Weather | 6 | 4 |
| Tone Out | 7 | 5 |

## Item codes (`DispOptItems`) — grouped by option area

`DispOptId` picks the screen area; the value written is the **File** token.

- **`DispOptId=1` — Huge area:** `Empty`, `CTCSS/DCS`, `FL_Name`, `Frequency`, `NumberTag`,
  `SysSubID`, `ServiceType`, `SiteId`, `SiteName`, `SystemType`, `SystemId`, `TGID`, `UnitId`,
  `UnitIdName`, `Volume&Squelch`, `WACN`.
- **`DispOptId=2` — Large area:** `Empty`, `BattVoltage`, `CTCSS/DCS`, `D_ErrorCount`,
  `Filter`, `FL_Name`, `Frequency`, `latitude`, `Lcn`, `longitude`, `Noise`, `NumberTag`,
  `SysSubID`, `Rssi`, `Rssi Bar`, `ServiceType`, `SiteId`, `SiteName`, `SystemType`,
  `SystemId`, `TdmaSlot`, `TGID`, `UnitId`, `UnitIdName`, `USB1_vbus`, `USB2_vbus`,
  `Volume&Squelch`, `WACN`, `Bluetooth`, `Battery Current`, `Battery Temperature`
  (`Bluetooth`/`Battery *`/USB Vbus are **SDS150 V2.00** additions).
- **`DispOptId=3` — Small area:** `Empty`, `ATT`, `SCR` (Broadcast Screen), `CC` (Close Call),
  `Day` (Date), `P25Status`, `GPS`, `IFX`, `Modulation`, `P_Ch`, `PRI` (Priority Scan), `REC`,
  `REP` (Repeater Find), `Squelch`, `TdmaSlot`, `Time`, `Volume`, `LVL` (Volume Offset),
  `WxPRI`.
- **`DispOptId=4` — Small area (lower row):** shares the small-area vocabulary above. (The V2.00
  spec sheet labels this the "Icon area," but a **real SDS150 card carries data-item tokens here**
  — e.g. `Modulation`, `P_Ch`, `IFX`, `LVL`, `REC`, `GPS`, `PRI`, `CC`, `REP`, `SCR`, `WxPRI` —
  not `ICON` slots; treat it as a second small-item row.)

## Color groups (`DispColors`)

`DispColorId` selects which UI elements a color row paints (each element = a `Text Back` pair):

| `DispColorId` | Paints |
|---|---|
| 1 | System Name, System Avoid, Dept Name, Dept Avoid, Channel Name, Channel Avoid |
| 2 | System Option, Dept Option, Channel Option |
| 3 | `Option_1`…`Option_5` (small-area items) |
| 4 | `Option A_1`, `Option_B_1` (large-area items) |
| 5 | `ICON1`…`ICON5` |
| 6 | `F`, `SIG`, `BAT`, `SP0`, `KEY` (status bar) |
| 7 | `Soft Key 1`, `SP1`, `Soft Key 2`, `SP2`, `Soft Key 3` |

## Color palette (`Color, Item code` sheet)

The allowed colors: a named palette (X11-style names with **Uniden's own hex values**, which
differ from standard web hex, e.g. `Aqua = 00fbf7`). A `DispColors` field is one of these hex
values (any 6-hex is likely accepted, but the UI should offer this set). 147 colors:

```
Aliceblue #eff7ff        Antiquewhite #f7ebd6     Aqua #00fbf7
Aquamarine #7bffce       Azure #efffff            Beige #eff3d6
Bisque #ffe3bd           Black #000000            Blanchedalmond #ffebc6
Blue #0000ff             Blueviolet #8429de       Brass #b5a542
Brown #a52929            Burlywood #d6b584        Cadetblue #5a9c9c
Chartreuse #7bff00       Chocolate #ce6718        Coolcopper #d68418
Copper #bd00de           Coral #ff7f4a            Cornflower #bdefde
Cornflowerblue #6390e7   Cornsilk #fff7d6         Crimson #d61039
Cyan #00ffff             Darkblue #000084         Darkbrown #d60800
Darkcyan #008884         Darkgoldenrod #b58408    Darkgray #a5a5a5
Darkgreen #006300        Darkkhaki #b5b56b        Darkmagenta #840084
Darkolivegreen #526b29   Darkorange #ff8800       Darkorchid #9431c6
Darkred #840000          Darksalmon #e79473       Darkseagreen #8cb98c
Darkslateblue #423d84    Darkslategray #294e4a    Darkturquoise #00cace
Darkviolet #8c00ce       Deeppink #ff108c         Deepskyblue #00bdff
Dimgray #636763          Dodgerblue #188cff       Feldspar #f7cede
Firebrick #ad2121        Floralwhite #fff7ef      Forestgreen #218821
Fuchsia #f700f7          Gainsboro #d6dad6        Ghostwhite #f7f7ff
Gold #ffd600             Goldenrod #d6a118        Gray #7b7f7b
Green #007f00            Greenyellow #adff29      Honeydew #efffef
Hotpink #ff67ad          Indianred #c65a5a        Indigo #4a007b
Ivory #ffffef            Khaki #efe38c            Lavender #dee3f7
Lavenderblush #ffefef    Lawngreen #7bfb00        Lemonchiffon #fff7c6
Lightblue #add6de        Lightcoral #ef7f7b       Lightcyan #deffff
Lightgoldenrodyellow #f7f7ce   Lightgreen #8ceb8c   Lightgray #ced2ce
Lightpink #ffb1bd        Lightsalmon #ff9c73      Lightseagreen #18ada5
Lightskyblue #84caf7     Lightslategray #738494   Lightsteelblue #adc2d6
Lightyellow #ffffde      Lime #00ff00             Limegreen #31ca31
Linen #f7efde            Magenta #ff00ff          Maroon #7b0000
Mediumaquamarine #63caa5 Mediumblue #0000c6       Mediumorchid #b556ce
Mediumpurple #8c6fd6     Mediumseagreen #39b16b   Mediumslateblue #7367e7
Mediumspringgreen #00f794 Mediumturquoise #42cec6 Mediumvioletred #c61484
Midnightblue #18186b     Mintcream #effff7        Mistyrose #ffe3de
Moccasin #ffe3b5         Navajowhite #ffdaad      Navy #00007b
Oldlace #f7f3de          Olive #7b7f00            Olivered #6b8c21
Orange #ffa100           Orangered #ff4600        Orchid #d66fd6
Palegoldenrod #e7e7a5    Palegreen #94fb94        Paleturquoise #adebe7
Palevioletred #d66f8c    Papayawhip #ffefce       Peachpuff #ffd6b5
Peru #c68039             Pink #ffbdc6             Plum #d69cd6
Powderblue #addede       Purple #7b007b           Red #ff0000
Richblue #08adde         Rosybrown #b58c8c        Royalblue #3967de
Saddlebrown #844610      Salmon #f77f6b           Sandybrown #efa15a
Seagreen #298852         Seashell #fff3e7         Sienna #9c5229
Silver #bdbdbd           Skyblue #84cae7          Slateblue #635ac6
Slategray #6b7f8c        Snow #fff7f7             Springgreen #00ff7b
Steelblue #4280ad        Tan #ceb18c              Teal #007f7b
Thistle #d6bdd6          Tomato #ff6342           Turquoise #39dece
Violet #e780e7           Wheat #efdaad            White #ffffff
Whitesmoke #eff3ef       Yellow #ffff00           Yellowgreen #94ca31
```

## Notes for building the feature

- **New file class.** The records live in **`profile.cfg`** (the model-folder settings config,
  alongside `app_data.cfg`/`discvery.cfg`) — confirmed against a real SDS150 card. `SdLayout`
  already names it (`profile_cfg`), so no new path hook is needed; `card::display_cfg_path`
  reuses it. `profile.cfg` also holds many non-display records (owner info, band defaults, tone
  out, GPS, …) — the writer touches only the four display records and preserves the rest verbatim.
- **Round-trip first.** Gate any writer behind the byte-exact round trip (read → decode →
  re-encode == original), exactly as for favorites; the records above have several `Reserve`
  fields to preserve verbatim (the "never overwrite what we don't know" rule).
- **UI shape.** A per-mode editor: pick a layout mode (the 7 above), assign item File tokens
  to each option area, and set text/back colors per element from the palette (a live preview
  mirroring the scanner's screen). The service-type colors we already ship can seed sensible
  defaults.
- **Scope.** Icons (`DispOptId=4`) and the SDS100/200-only backlight fields can be a later
  pass; the text items + colors are the high-value core.
