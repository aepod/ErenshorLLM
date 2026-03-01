# Item Lore Text and Descriptions

> Extracted from decompiled Erenshor source code.
> Item descriptions (the `Lore` field on `Item` ScriptableObjects) are stored as Unity asset data,
> not as hardcoded strings in the C# source. They are set in the Unity Editor on each Item
> ScriptableObject and serialized into asset bundles. The decompiled C# code only defines the
> field -- it does not contain the actual per-item lore text.

## Item System Architecture

> Source: `decompiled/project/Item.cs` lines 125-126

The `Item` class (ScriptableObject) has a `Lore` field defined as:

```csharp
[TextArea(5, 20)]
public string Lore;
```

This field holds the item's flavor text / description shown in the item info tooltip.
The actual text values are baked into Unity's serialized `.asset` files and are NOT present
in the decompiled C# code. To extract them, one would need to use a Unity asset extractor
(e.g., AssetStudio, UABE) on the game's data files.

## Item Display System

> Source: `decompiled/project/ItemInfoWindow.cs` lines 238-241

When displaying an item tooltip, the game shows:
- Item name
- Stat block (STR, END, DEX, AGI, INT, WIS, CHA, resists, AC, HP, Mana)
- Weapon damage and delay (if applicable)
- The `Lore` field text
- Class restrictions
- Slot type
- Special effects (worn effects, weapon procs, click effects, auras)
- Item value (gold)

Items can also have a `BookTitle` field (string) which associates them with in-game books
from the `AllBooks` dictionary.

## Item Quality Tiers

Items can have quality levels:
- Normal (white text)
- Blessed (quality 2, special text color)
- Legendary/Godly (quality 3, special text color)

Quality modifiers scale stats, damage, AC, HP, and Mana according to formulas in
`Item.CalcStat()`, `Item.CalcDmg()`, `Item.CalcRes()`, and `Item.CalcACHPMC()`.

## Special Item Types

> Source: `decompiled/project/Item.cs`

- **Relic Items**: `bool Relic` -- Special rare items
- **Unique Items**: `bool Unique` -- Only one can be held
- **No Trade/No Destroy**: `bool NoTradeNoDestroy` -- Cannot be sold or destroyed
- **Template Items**: `bool Template` -- Crafting recipe items with ingredient lists
- **Stackable Items**: `bool Stackable` -- Can stack in inventory
- **Disposable Items**: `bool Disposable` -- Consumed on use
- **Furniture Set Items**: `bool FurnitureSet` -- Housing items
- **Fuel Source Items**: `bool FuelSource` with `FuelTier FuelLevel` (1-5)

## Charm Items

> Source: `decompiled/project/ItemInfoWindow.cs` lines 264-307

Charm items (Slot: Charm) have special scaling modifiers instead of flat stats:
- Physicality (StrScaling)
- Hardiness (EndScaling)
- Finesse (DexScaling)
- Defense (AgiScaling)
- Arcanism (IntScaling)
- Restoration (WisScaling)
- Mind (ChaScaling)
- Resist Scaling
- Mitigation Scaling

The tooltip explains: "Charms do not increase stats, they modify how effectively your character utilizes stats."

## Aura Items

> Source: `decompiled/project/ItemInfoWindow.cs` lines 559-564

Aura items display: "Aura Item -- Auras effect entire party -- Auras of same type do not stack"
followed by the aura spell name and description.
