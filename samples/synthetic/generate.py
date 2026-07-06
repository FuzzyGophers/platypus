#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 The Platypus Authors
"""
Generate privacy-safe SYNTHETIC SDSx00 fixtures for tests.

These are entirely fictional (made-up state "Example State", counties Alpha/Bravo/
Cedar, neutral coordinates) so the repo never embeds anyone's real location. They
reproduce the real on-card structure (record widths, column positions, dialects)
validated earlier, so they exercise the same code paths as real card data.

Run from the repo root or this dir:  python3 samples/synthetic/generate.py
Outputs (CRLF, tab-delimited, ASCII):
  s_example.hpd   multi-county/agency state HPDB (for extraction tests)
  f_example.hpd   favorites file incl. DQKs_Status / BandPlan_P25 / Rectangle
  hpdb.cfg        small county master (for CountyIndex)
  f_list.cfg      favorites index (F-List)
  profile.cfg     settings/display config (DisplayOption/Backlight/DispOptItems/DispColors)
"""

import os

OUT = os.path.dirname(os.path.abspath(__file__))
CRLF = "\r\n"

# Fictional identifiers used by tests. Keep in sync with tests/*.rs.
STATE_ID = 90
ALPHA, BRAVO, CEDAR = 9001, 9002, 9003  # CountyIds
# Neutral coordinates (rural high plains; not anyone's home).
ALPHA_LL = (45.000000, -100.000000)
BRAVO_LL = (46.000000, -101.000000)
HUB_LL = (45.500000, -100.500000)
FAR_LL = (48.000000, -110.000000)

# Standard P25 band plan (a public constant, not location data).
P25_BANDPLAN = [
    "", "851006250", "6250", "762006250", "6250", "851012500", "12500",
    "762006250", "12500", "935012500", "12500", "935012500", "12500",
] + ["0"] * 22 + ["3", "3", "0", "3"] + ["0"] * 10


def rec(width, cols):
    """A record of exactly `width` tab fields; cols maps index->value."""
    row = [""] * width
    for i, v in cols.items():
        row[i] = str(v)
    return row


def write(name, rows):
    text = CRLF.join("\t".join(r) for r in rows) + CRLF
    path = os.path.join(OUT, name)
    with open(path, "wb") as f:
        f.write(text.encode("ascii"))
    print(f"wrote {name}: {len(rows)} lines, {len(text)} bytes")


def header(extra=None):
    rows = [["TargetModel", "BCDx36HP"], ["FormatVersion", "1.00"]]
    if extra:
        rows.append(extra)
    return rows


# ---- s_example.hpd : full HPDB dialect, explicit ids + area tags ----
def state_file():
    r = header()

    # System 1 — county-organized Conventional (AreaCounty field1 == field2).
    r.append(rec(15, {0: "Conventional", 1: f"CountyId={ALPHA}",
                        2: f"StateId={STATE_ID}", 3: "Alpha County Public Safety",
                        6: "Conventional"}))
    r.append(rec(3, {0: "AreaState", 1: f"CountyId={ALPHA}", 2: f"StateId={STATE_ID}"}))
    r.append(rec(3, {0: "AreaCounty", 1: f"CountyId={ALPHA}", 2: f"CountyId={ALPHA}"}))
    r.append(rec(11, {0: "C-Group", 1: "CGroupId=8001", 2: f"CountyId={ALPHA}",
                        3: "Alpha Fire", 5: f"{ALPHA_LL[0]:.6f}", 6: f"{ALPHA_LL[1]:.6f}",
                        7: "5.0", 8: "Circle", 10: "Global"}))
    # col 6 = mode, col 8 = RadioReference service-type code (3 = Fire Dispatch).
    r.append(rec(18, {0: "C-Freq", 1: "CFreqId=7001", 2: "CGroupId=8001",
                        3: "Alpha Dispatch", 5: "154100000", 6: "NFM",
                        7: "TONE=C156.7", 8: "3"}))

    # System 2 — agency-organized Conventional. AreaCounty field1 (AgencyId) !=
    # field2 (CountyId). This is the case that exposed the field-2 county rule.
    r.append(rec(15, {0: "Conventional", 1: "AgencyId=9101",
                        2: f"StateId={STATE_ID}", 3: "Regional Transit",
                        6: "Conventional"}))
    r.append(rec(3, {0: "AreaState", 1: "AgencyId=9101", 2: f"StateId={STATE_ID}"}))
    r.append(rec(3, {0: "AreaCounty", 1: "AgencyId=9101", 2: f"CountyId={BRAVO}"}))
    r.append(rec(11, {0: "C-Group", 1: "CGroupId=8002", 2: "AgencyId=9101",
                        3: "Transit Ops", 5: f"{BRAVO_LL[0]:.6f}", 6: f"{BRAVO_LL[1]:.6f}",
                        7: "8.0", 8: "Circle", 10: "Global"}))
    r.append(rec(18, {0: "C-Freq", 1: "CFreqId=7002", 2: "CGroupId=8002",
                        3: "Transit Net", 5: "453200000", 6: "NFM", 8: "22"}))  # Transportation

    # System 3 — P25 trunk spanning two counties; has Circle + Rectangles groups.
    r.append(rec(22, {0: "Trunk", 1: "TrunkId=9201", 2: f"StateId={STATE_ID}",
                        3: "Example Statewide P25", 6: "P25Standard"}))
    r.append(rec(3, {0: "AreaState", 1: "TrunkId=9201", 2: f"StateId={STATE_ID}"}))
    r.append(rec(3, {0: "AreaCounty", 1: "TrunkId=9201", 2: f"CountyId={ALPHA}"}))
    r.append(rec(3, {0: "AreaCounty", 1: "TrunkId=9201", 2: f"CountyId={CEDAR}"}))
    r.append(rec(20, {0: "Site", 1: "SiteId=8201", 2: "TrunkId=9201", 3: "Central Site",
                        5: f"{HUB_LL[0]:.6f}", 6: f"{HUB_LL[1]:.6f}", 7: "10.0", 11: "Circle"}))
    r.append(rec(9, {0: "T-Freq", 1: "TFreqId=6001", 2: "SiteId=8201", 4: "851012500"}))
    r.append(rec(20, {0: "Site", 1: "SiteId=8202", 2: "TrunkId=9201", 3: "Far Site",
                        5: f"{FAR_LL[0]:.6f}", 6: f"{FAR_LL[1]:.6f}", 7: "10.0", 11: "Circle"}))
    r.append(rec(9, {0: "T-Freq", 1: "TFreqId=6002", 2: "SiteId=8202", 4: "851025000"}))
    r.append(rec(10, {0: "T-Group", 1: "TGroupId=8301", 2: "TrunkId=9201", 3: "Dispatch",
                        5: f"{HUB_LL[0]:.6f}", 6: f"{HUB_LL[1]:.6f}", 7: "10.0", 8: "Circle"}))
    # col 6 = audio (DIGITAL/ANALOG/ALL), col 7 = service-type code (2 = Law Dispatch).
    r.append(rec(17, {0: "TGID", 1: "Tid=5001", 2: "TGroupId=8301", 3: "Police",
                       5: "101", 6: "DIGITAL", 7: "2"}))
    # Rectangles-shaped group: lat/lon 0/0, followed by bounding boxes.
    r.append(rec(10, {0: "T-Group", 1: "TGroupId=8302", 2: "TrunkId=9201", 3: "Regionwide",
                        5: "0.000000", 6: "0.000000", 7: "0", 8: "Rectangles"}))
    r.append(rec(6, {0: "Rectangle", 2: "47.000000", 3: "-103.000000",
                       4: "44.000000", 5: "-98.000000"}))
    r.append(rec(6, {0: "Rectangle", 2: "44.000000", 3: "-103.000000",
                       4: "43.000000", 5: "-99.000000"}))
    r.append(rec(17, {0: "TGID", 1: "Tid=5002", 2: "TGroupId=8302", 3: "Regional",
                       5: "102", 6: "ANALOG", 7: "8"}))  # Fire-Tac

    # System 4 — non-P25 (MotoTrbo) trunk, tagged to Bravo county.
    r.append(rec(22, {0: "Trunk", 1: "TrunkId=9202", 2: f"StateId={STATE_ID}",
                        3: "Example Business Radio", 6: "MotoTrbo"}))
    r.append(rec(3, {0: "AreaState", 1: "TrunkId=9202", 2: f"StateId={STATE_ID}"}))
    r.append(rec(3, {0: "AreaCounty", 1: "TrunkId=9202", 2: f"CountyId={BRAVO}"}))
    r.append(rec(20, {0: "Site", 1: "SiteId=8203", 2: "TrunkId=9202", 3: "Business Site",
                        5: f"{BRAVO_LL[0]:.6f}", 6: f"{BRAVO_LL[1]:.6f}", 7: "3.0", 11: "Circle"}))
    r.append(rec(9, {0: "T-Freq", 1: "TFreqId=6003", 2: "SiteId=8203", 4: "461000000"}))
    r.append(rec(10, {0: "T-Group", 1: "TGroupId=8303", 2: "TrunkId=9202", 3: "Ops",
                        5: f"{BRAVO_LL[0]:.6f}", 6: f"{BRAVO_LL[1]:.6f}", 7: "3.0", 8: "Circle"}))
    r.append(rec(17, {0: "TGID", 1: "Tid=5003", 2: "TGroupId=8303", 3: "Channel 1", 5: "1"}))

    # Named by StateId (90) so the card folder loader can resolve it from a county.
    write("s_000090.hpd", r)


# ---- f_example.hpd : favorites dialect (blanked ids, no area tags),
#      including the favorites-only records DQKs_Status and BandPlan_P25. ----
def favorites_file():
    dqks = rec(102, {0: "DQKs_Status"})
    for i in range(2, 102):
        dqks[i] = "Off"
    bandplan = ["BandPlan_P25"] + P25_BANDPLAN

    r = header()
    # Conventional system (ids blanked) with a Rectangles group.
    r.append(rec(15, {0: "Conventional", 3: "Alpha County Public Safety", 6: "Conventional"}))
    r.append(list(dqks))
    r.append(rec(11, {0: "C-Group", 3: "Regionwide", 5: "0.000000", 6: "0.000000",
                        7: "0", 8: "Rectangles", 10: "Global"}))
    r.append(rec(6, {0: "Rectangle", 2: "47.000000", 3: "-103.000000",
                       4: "44.000000", 5: "-98.000000"}))
    r.append(rec(18, {0: "C-Freq", 3: "Alpha Dispatch", 5: "154100000", 6: "NFM"}))
    # P25 trunk system (ids blanked); site carries a synthesized BandPlan_P25.
    r.append(rec(22, {0: "Trunk", 3: "Example Statewide P25", 6: "P25Standard"}))
    r.append(list(dqks))
    r.append(rec(20, {0: "Site", 3: "Central Site", 5: f"{HUB_LL[0]:.6f}",
                        6: f"{HUB_LL[1]:.6f}", 7: "10.0", 11: "Circle"}))
    r.append(list(bandplan))
    r.append(rec(9, {0: "T-Freq", 4: "851012500"}))
    r.append(rec(10, {0: "T-Group", 3: "Dispatch", 5: f"{HUB_LL[0]:.6f}",
                        6: f"{HUB_LL[1]:.6f}", 7: "10.0", 8: "Circle"}))
    r.append(rec(17, {0: "TGID", 3: "Police", 5: "101"}))

    write("f_example.hpd", r)


# ---- hpdb.cfg : small synthetic county master ----
def hpdb_file():
    r = header(["DateModified", "01/01/2020 00:00:00"])
    r.append(rec(5, {0: "StateInfo", 1: f"StateId={STATE_ID}", 2: "CountryId=0",
                       3: "Example State"}))
    for cid, name in [(ALPHA, "Alpha"), (BRAVO, "Bravo"), (CEDAR, "Cedar")]:
        r.append(rec(4, {0: "CountyInfo", 1: f"CountyId={cid}",
                           2: f"StateId={STATE_ID}", 3: name}))
    # A couple of Locate-Me index rows.
    r.append(rec(9, {0: "LM", 1: f"StateId={STATE_ID}", 2: f"CountyId={ALPHA}",
                       3: "TrunkId=9201", 4: "SiteId=8201",
                       7: f"{HUB_LL[0]:.6f}", 8: f"{HUB_LL[1]:.6f}"}))
    write("hpdb.cfg", r)


# ---- f_list.cfg : favorites index ----
def flist_file():
    r = header()
    fl = rec(118, {0: "F-List", 1: "Example List", 2: "f_example.hpd",
                     3: "Off", 4: "On", 5: "0"})
    for i in range(6, 118):
        fl[i] = "Off"
    r.append(fl)
    write("f_list.cfg", r)


# ---- profile.cfg : settings/display config (the display-customization file class) ----
def profile_file():
    # A rich settings file in reality; here just enough to exercise the four display records
    # (DisplayOption / Backlight / DispOptItems / DispColors) plus one non-display record
    # (BandDefault) to prove the display module preserves everything it doesn't own. No owner
    # or location data. Column positions match a real SDS150 card.
    r = header()
    r.append(rec(5, {0: "BandDefault", 1: "1", 2: "25000000", 3: "54000000", 4: "AM"}))
    r.append(rec(14, {0: "DisplayOption", 6: "DEC", 11: "On", 12: "AFS", 13: "COLOR"}))
    r.append(rec(12, {0: "Backlight", 2: "High", 5: "30", 6: "40",
                        7: "Off", 8: "Off", 9: "On", 10: "5", 11: "Infinite"}))
    r.append(["DispOptItems", "DispOptId=1", "DispLayoutId=1", "FL_Name", "Empty", "Frequency"])
    r.append(["DispOptItems", "DispOptId=3", "DispLayoutId=1", "ATT", "Bluetooth", "Day"])
    r.append(["DispColors", "DispColorId=1", "ColorLayoutId=1",
                "ff4600", "000000", "ff8800", "000000"])
    r.append(["DispColors", "DispColorId=4", "ColorLayoutId=1", "e79473", "000000"])
    write("profile.cfg", r)


if __name__ == "__main__":
    state_file()
    favorites_file()
    hpdb_file()
    flist_file()
    profile_file()
