---
title: "Haste"
source: "wiki"
wiki_source: "https://erenshor.wiki.gg/wiki/Haste"
categories: []
lore_category: "misc"
---

# Haste

# Haste

**Haste** – The higher it is, the faster you attack.

Haste increases the rate at which your character performs basic attacks by reducing attack delay. It does not increase damage per hit directly, but by lowering weapon delay it increases overall DPS.

## Sources of Haste

### Auras

_Only one Aura may be active at a time._

Aura: Haste: Class: Replaced by
Scent of the Sea: 3%: Windblade: Force of the Sea
Force of the Sea: 6%: Windblade: Freedom of the Sky
Freedom of the Sky: 9%: Windblade: Whispers of Wind
Whispers of Wind: 12%: Windblade
Hallows Eve: 8%: All
Wisp's Presence: 5%: All: Hallows Eve

* * *

### Temporary Buffs

Buff: Haste: Duration
Fernalla's Presence: 10%: 15s
Hydrated: 3%: 498s
Spiced Fury: 15%: 90s
Theft of Vigor: 10%: 15s

#### Reaver Haste

Buff: Haste Scaling: Replaced by
Affinity for Suffering: 2% per 10% missing HP: Quest for Suffering
Quest for Suffering: 4% per 10% missing HP

* * *

### Worn Item Haste

_Active while the item is equipped._

Effect: Haste: Replaced by
Contagious Rage: 5%
Dark Haste: 8%: Dark Haste II
Dark Haste II: 15%
Embrace of Shadow: 5%
Flow: 13%: Ocean's Kiss
Hint of Shadow: 5%: Embrace of Shadow
Ocean's Kiss: 17%
Slime Flow: 10%: Flow
Vampirism: 5%
Vitheo's Blessing of the Wind: 10%
Windblessed: 15%: Ocean's Kiss, Flow

## Special Interactions

* Dark Haste II does **not** stack with Seaspice's Spiced Fury.

## Mechanics

### Stacking

All haste from status effects is **additive**. Total haste is summed together before being applied.

### Haste & Slow Caps

The game enforces the following limits:
code
if (seWeapHaste > 60)
seWeapHaste = 60;
if (seWeapHaste < -75)
seWeapHaste = -75;

/code

* Maximum Haste: **60%**
* Maximum Slow: **-75%**

The typical practical cap _without_ Auras is ' _43%_ , but you can reach 47% with Ceto.

Example worn item combination: Contagious Rage, Dark Haste II, Embrace of Shadow, Flow and Vampirism.

If a Windblade using Whispers of Wind is in your group and another member uses Hallows Eve or Wisp's Presence, you can reach **60%**.

The Reaver class is the only exception and can exceed the 60% haste cap through Affinity for Suffering or Quest for Suffering.

### Attack Delay Formula

Haste reduces total attack delay by a percentage of base delay.
code
Current Delay = Base Delay - (Base Delay × Haste%)

/code

Which can also be written as:
code
Current Delay = Base Delay × (1 - Haste / 100)

/code

There are no diminishing returns. Haste scales linearly.

### Example Calculation

Assume a weapon with a base delay of **3.0 seconds**.

Total Haste: Calculation: Final Delay (Seconds)
0%: 3.0 × (1 - 0.00): 3.00s
20%: 3.0 × (1 - 0.20): 2.40s
40%: 3.0 × (1 - 0.40): 1.80s
60%: 3.0 × (1 - 0.60): 1.20s

At 60% haste, a 3.0 second weapon swings every 1.2 seconds.

Because haste reduces delay linearly, there are no diminishing returns. Increasing haste from 40% to 60% results in a significant reduction in swing time.

* * *

Combat Statistics
Haste • Resonance