# Erenshor World Lore -- Extracted from Source Code

> Compiled from decompiled Erenshor source code.
> This document contains all narrative content, world-building details, and lore references
> found hardcoded in the game's C# source files.

---

## The World of Amarion

Amarion is the planet created by the Elder Gods Sivakaya and Brax. The continent of Erenshor
is one of at least two major landmasses, the other being Merosavilla (referenced as having
docks, a government, and settlers who came to Erenshor).

### The Pantheon

From the books and code, the divine hierarchy is:

| Deity/Entity | Role | Status |
|---|---|---|
| **Sivakaya** | Elder Goddess, co-creator of Amarion | Corrupted; retreated north |
| **Brax** | Elder God, co-creator of Amarion | Withdrew from the world in despair |
| **Vitheo** | Child of Brax and Sivakaya | Fell in war, ascended to divinity |
| **Fernalla** | Child of Brax and Sivakaya | Fell in war/treachery, ascended to divinity |
| **Soluna** | Cosmic entity, "fell from the stars" | Withdrew after war with Astra |
| **Astra** | Cosmic entity, opposed Soluna | Scattered into stardust, reborn from despair |
| **Azynthi** | Mortal sorcerer turned corrupted being | Absorbed Sivakaya's essence; fate unclear |

### The Timeline (from "Godless Amarion" and "Wardens of the Northern Veil")

1. Sivakaya and Brax create Amarion, burying a "balancing darkness" in the planet's crust
2. Humanity thrives on the surface; the North remains uninhabited and dangerous
3. Sivakaya secretly sends guardians (future Reavers) to watch the northern Void
4. Sivakaya teaches magic at the Duskenlight school
5. Azynthi, a gifted student, travels north seeking forbidden knowledge
6. Azynthi returns changed; retreats to an island east of Braxonia
7. Sivakaya, foreseeing a threat, accepts Azynthi's offer of power against Brax's warning
8. The ritual strips Sivakaya of her essence; a monolith of corruption forms
9. Sivakaya vanishes for decades with followers
10. The monolith spreads corruption across the land
11. Sivakaya returns in a twisted, demonic form with corrupted followers
12. Brax smites the land, creating the Braxonian Desert to contain the blight, burying his city
13. "The Wisp" (led by Eldrin Shadowmire) breaks from Sivakaya, retreats to Loomingwood Forest
14. Brax gives Eldrin the Nighthollow Candle to illuminate Sivakaya's past memories
15. War between Braxonians and corrupted Sivakayans
16. Vitheo and Fernalla fight alongside Brax against their mother Sivakaya
17. Braxonian victory, but Vitheo and Fernalla are killed (ascend to divinity)
18. Brax turns away from the world in grief
19. Nearly five centuries pass
20. Soluna arrives -- sky catches fire, stone flung into the sky (creating Soluna's Landing crater)
21. Soluna forcibly reshapes the Reavers, stripping their Void connection
22. Reavers refuse to serve Soluna; she retaliates
23. Some weak Reavers submit, becoming the foundation of the Paladin order
24. Astra arrives, wars with Soluna in the heavens
25. Soluna withdraws; Astra scatters into stardust at the crater's edge
26. The Solunarian Paladins discover the crater (Soluna's Landing) and begin building Port Azure
27. Port Azure construction: 1552-1700
28. Merosavilla government takes administrative control of Port Azure
29. The Azure Guard is established

---

## Geographic Locations

> Source: `decompiled/project/GetCommonTerms.cs` (zone name mappings)

| Scene Name | Display Name |
|---|---|
| Hidden | Hidden Hills |
| Brake | Faerie's Brake |
| Vitheo | Vitheo's Watch |
| FernallaField | Fernalla's Field |
| Bonepits | The Bonepits |
| Azure | Port Azure |
| Elderstone | Elderstone Mines |
| SaltedStrand | Blacksalt Strand |
| VitheosEnd | Vitheo's Rest |
| Ripper | Ripper's Keep |
| PrielPlateau | Prielian Cascade |
| Abyssal | Abyssal Lake |
| Tutorial | Island Tomb |
| Stowaway | Stowaway's Step |
| ShiveringStep | Shivering Step |
| AzynthiClear | Azynthi's Garden |
| Soluna | Soluna's Landing |
| Malaroth | Malaroth Nesting Grounds |
| Braxonian | Braxonian Desert |
| Braxonia | Fallen Braxonia |
| Loomingwood | (implied from code, home of The Wisp) |
| Willowwatch | (alternate starting zone) |
| SummerEvent | (seasonal event zone) |
| ShiveringTomb | (dungeon) |
| ShiveringTomb2 | (dungeon, deeper level) |

### Notable Locations from Lore

- **Duskenlight** -- Forest zone; location of Sivakaya's school where magic was first freely taught
- **Silkengrass Meadow** -- Where the Albino Kodiak has been sighted
- **Twin Soldiers** -- Mountain pass marking the border of the dangerous North
- **Rockshade Hold** -- Ruined monastery; monks turned to stone by Soluna's Landing impact
- **Loomingwood Forest** -- Home of The Wisp, warded against Sivakaya's presence
- **Braxonian Desert** -- Created by Brax destroying his own city to contain corruption
- **Fallen Braxonia** -- Ruins of Brax's buried city beneath the desert
- **Soluna's Landing** -- Crater from Soluna's arrival; site of spiritual significance
- **Malaroth Nesting Grounds** -- Where the Malaroth beasts are found
- **Azynthi's Garden** -- Named after the corrupted sorcerer
- **Abyssal Lake** -- Referenced in Benjamin's Journal as location of interest

---

## Factions

> Source: `decompiled/project/WorldFaction.cs` and `decompiled/project/GlobalFactionManager.cs`

Factions are ScriptableObjects with:
- `FactionName`: Display name
- `FactionDesc`: Description (shown in "gained/lost standing with {FactionDesc}")
- `REFNAME`: Internal reference key
- `FactionValue`: Current standing (float, negative = hostile)
- `DEFAULTVAL`: Starting value

The actual faction names and descriptions are serialized in Unity assets.

### Faction-Related Spell Lines

> Source: `decompiled/project/Spell.cs` lines 94-99

Spell lines reference specific factions/cultures:
- `Vithean_Buff`
- `Solunarian_Buff`
- `Braxonian_Buff`
- `Sivakayan_Buff`
- `Azynthian_Buff`
- `Fernallan_Buff`

---

## Character Classes

> Source: `decompiled/project/CharSelectManager.cs` lines 43-59

### Paladin

> Source: line 44

"The Paladin is a master of weapon combat and excels at using heavy armor and weapon types.

To sustain in battle, a Paladin can depend on Solunarian Magic of the Day and Night.

A Paladin should focus on Strength, Agility, and Endurance."

### Windblade (Duelist)

> Source: line 47
> Note: Internal class name is "Duelist" but display name is "Windblade"

"The Duelist is a master of offensive combat and is best while using light armor and weapon types.

Duelists thrive on combat. They gain health by dealing damage, and excel at crippling their opponents' abilities.

A Duelist should focus on Dexterity, Strength, and Intelligence."

### Druid

> Source: line 50

"The Druid is a master of all things nature - both pleasant and unpleasant.

The Druid can call upon not only the force of life, but he also commands the power of death.

A Druid should focus on Intelligence, Wisdom, and Charisma."

### Arcanist

> Source: line 53

"The Arcanist is a magical being, who thrives in cloth armor.

Arcanists command magic of all types, and they have many tools to avoid close physical combat.

An Arcanist should focus on Intelligence, Wisdom, and Charisma."

### Stormcaller

> Source: line 56
> Note: Description text is stored in serialized field `StormcallerClassDesc`, not hardcoded in C#

(Description set via Unity Inspector, not in decompiled source)

### Reaver

> Source: line 58
> Note: Description text is stored in serialized field `ReaverClassDesc`, not hardcoded in C#

(Description set via Unity Inspector, not in decompiled source)

---

## The Sivakayan Spectres

> Source: `decompiled/project/ZoneAnnounce.cs` lines 179-231

When the player possesses the "Watcher's Lens" cosmetic item, Sivakayan Spectres can
randomly spawn in most zones (excluded: Azure, Stowaway, Tutorial, SummerEvent,
ShiveringStep, ShiveringTomb, ShiveringTomb2). Each zone can only spawn one spectre
per session. There is a 10% chance of spawning when conditions are met.

---

## Memory Spheres

> Source: `decompiled/project/MemorySphere.cs`

Memory Spheres are in-world objects that reveal lore text when the player approaches them
while carrying a specific required item. The `Lore` field (string) contains the text displayed
in the social log. The actual text is set in the Unity scene data per sphere instance.

---

## The Reliquary

Referenced throughout the code (`GameData.ReliqDest`, `ReliqDisableFiendSpawn.cs`,
`ReliquaryFiend.cs`), the Reliquary appears to be a special zone or mechanic. Benjamin's
Journal hints at "a place that can bestow the touch of a God upon any ordinary item" which
may reference this system.

---

## Loading Screen / Main Menu Flavor Text

> Source: `decompiled/project/MainMenu.cs` lines 34, 493-498

The main menu has a `Flavor` list (List<string>) that cycles through flavor text lines
during the loading sequence. These are serialized in the Unity scene, not in the C# code.
The loading sequence shows up to 8 flavor text entries at random intervals.

---

## Creatures of Note (from Lore)

### Malaroths

> Source: "The Malaroth Ledger" book

- Fury wrapped in muscle; worth more gold than most people see in a lifetime
- Found in the Malaroth Nesting Grounds
- Can be fed Moongill fish; this attracts "a darker sort of beast" with eyes sharper and movements more deliberate
- The afflicted ones (touched by madness) resemble Sivakayan zealots
- May have been shaped by Sivakaya's corruption
- Savannah Priel and her crew attempted to capture and export them to Merosavilla

### Albino Kodiak

> Source: "Strange Beasts of Erenshor" book

- Spectral predator seen in Silkengrass Meadow
- Whispered to bring luck or doom depending on the intent of the observer

### Giant Men

> Source: "Strange Beasts of Erenshor" book

- Found in Duskenlight forests
- Tread softly but leave imprints the size of shields

### The Beast of The Brake

> Source: "Strange Beasts of Erenshor" book

- Described as "a demon of flesh and shadow"
- Roams The Brake (Faerie's Brake) under pale moonlight
- Lingers near the swampy heart of the woods

---

## Named Characters (from Lore)

| Name | Role | Source |
|---|---|---|
| **Sivakaya** | Elder Goddess, co-creator of Amarion | "Godless Amarion" |
| **Brax** | Elder God, co-creator of Amarion | "Godless Amarion" |
| **Azynthi** | Gifted sorcerer, corrupted by the Void | "Godless Amarion", "Wardens" |
| **Vitheo** | Demigod child of Brax and Sivakaya | "Godless Amarion" |
| **Fernalla** | Demigod child of Brax and Sivakaya | "Godless Amarion" |
| **Soluna** | Cosmic entity from the stars | "Wardens of the Northern Veil" |
| **Astra** | Cosmic entity who fought Soluna | "Wardens of the Northern Veil" |
| **Eldrin Shadowmire** | Leader of The Wisp | "Godless Amarion" |
| **Savannah Priel** | Captain, duelist, Malaroth hunter | "The Malaroth Ledger" |
| **Benjamin** | Treasure hunter searching for divine crafting site | "Benjamin's Journal" |
| **GM-Burgee** | In-game GM (fictional staff member) | JailTimer.cs |

### Named NPCs from Code

> Source: `decompiled/project/NPCDialogManager.cs` lines 80-87

- **Thella Steepleton** -- Auction House NPC
- **Goldie Retalio** -- Auction House NPC
- **Prestigio Valusha** -- Banker NPC
- **Validus Greencent** -- Banker NPC
- **Comstock Retalio** -- Banker NPC
- **Summoned: Pocket Auctions** -- Portable auction house
- **Summoned: Pocket Rift** -- Portable bank

---

## Cosmological Concepts

### The Void

The Void is a dimension of darkness that bleeds through "unseen seams" in the North of
Erenshor. It is the source of the Reavers' original power and what corrupted Azynthi.
It cannot be driven back or denied -- only coexisted with in careful balance.

### The Balancing Darkness

When creating Amarion, Sivakaya and Brax buried a "balancing darkness deep within the
planet's crust" to allow prosperity on the surface. The North, where this darkness was
closest to the surface, remained uninhabited.

### The Monolith of Corruption

Formed during Azynthi and Sivakaya's ritual from the sheer energy of the disturbance.
It leaked corruption across the land like "flowing mud" until Brax destroyed it by
creating the Braxonian Desert.

### The Nighthollow Candle

A candle lit by Brax himself, given to Eldrin Shadowmire of The Wisp. It illuminates
past memories of Sivakaya's uncorrupted self and may hold the key to reversing her corruption.

### Stardust / Astra's Rebirth

When Astra was defeated by Soluna, she scattered into stardust at the crater's edge.
The despair of the trapped Reavers fed her rebirth. Anyone who left the mountains was
"drawn into her, consumed to restore what she had lost."

---

## Religious/Cultural Groups

| Group | Alignment | Description |
|---|---|---|
| **Solunarian Paladins** | Soluna | Founded from weak Reavers who submitted to Soluna |
| **The Wisp** | Anti-Sivakaya restoration | Led by Eldrin Shadowmire; seeks to restore Sivakaya |
| **Sivakayans** | Corrupted Sivakaya | Worship the corrupted goddess in darkened churches |
| **Braxonians** | Brax | Brax's people, buried beneath the desert |
| **Fernallan Druids** | Fernalla | Nature-focused, resided in temporary structures |
| **Vitheans** | Vitheo | Referenced in settlement and culture |
| **The Reavers** | Neutral/Void | Original guardians of the Northern Void |
| **Azure Guard** | Port Azure | Law enforcement of Port Azure |
| **Merosavilla Government** | Secular | Administered Port Azure |

---

## Damage/Magic Types

> Source: `decompiled/project/GameData.cs` (DamageType enum), `decompiled/project/ItemInfoWindow.cs` lines 809-819

| Type | Color Code | Lore Implication |
|---|---|---|
| Physical | White (#FFFFFF) | Mundane damage |
| Magic | Blue (#8080FF) | Arcane arts |
| Elemental | Orange (#FFA500) | Natural forces |
| Poison | Green (#50C878) | Toxic/nature corruption |
| Void | Purple (#B030B0) | The Void, connected to the North and corruption |
