# Quest System Text

> Extracted from decompiled Erenshor source code.

---

## Quest Architecture

> Source: `decompiled/project/Quest.cs`

Quests are `ScriptableObject` assets with the following text fields:

```csharp
public string QuestName;

[TextArea(10, 15)]
public string QuestDesc;         // Full quest description shown in journal

[TextArea(3, 5)]
public string DialogOnSuccess;   // NPC dialog when quest is completed

[TextArea(3, 5)]
public string DialogOnPartialSuccess;  // NPC dialog for partial completion

public string DisableText;       // Text shown if quest is disabled
```

**Important**: The actual quest text content (QuestDesc, DialogOnSuccess, etc.) is stored
as serialized data on Unity ScriptableObject `.asset` files, NOT in the decompiled C# code.
The C# code defines the schema only. To extract the actual quest text, a Unity asset
extractor would be needed.

## Quest System Details

- Quests are identified by `DBName` (string) for save/load
- Quests can be repeatable (`bool repeatable`)
- Quests require specific items (`List<Item> RequiredItems`)
- Completing quests grants XP (`int XPonComplete`), items (`Item ItemOnComplete`), and gold (`int GoldOnComplete`)
- Quests can chain: `Quest AssignNewQuestOnComplete`
- Quests can complete other quests: `List<Quest> CompleteOtherQuests`
- Quests affect faction standings: `List<WorldFaction> AffectFactions` with `List<float> AffectFactionAmts`
- Quests can set achievements: `string SetAchievementOnGet`, `string SetAchievementOnFinish`
- Quests can unlock vendor items: `Item UnlockItemForVendor`
- Special flags: `KillTurnInHolder`, `DestroyTurnInHolder`, `DropInvulnOnHolder`, `OncePerSpawnInstance`

## Quest Journal UI

> Source: `decompiled/project/QuestLog.cs`

The quest journal shows:
- Active quests tab
- Completed quests tab (non-repeatable)
- Repeatable quests tab
- 7 quests per page with pagination
- Clicking a quest shows its `QuestDesc` field

When no quest is selected: "No quest selected."
When a slot has no quest: "No Quest Assigned"

## Quest Delivery via NPC Dialog

> Source: `decompiled/project/NPCDialog.cs` lines 44-87

When an NPC gives/completes a quest through dialog:
- If quest is assigned and player lacks the required item: item is given with message "Received {ItemName} from {NPCName}"
- If player already has the required item: "You already have the item! ({ItemName})"
- If quest is already completed (non-repeatable): Shows `RepeatingQuestDialog` text
- If quest completion fails: "QUEST ERROR UH OH!" (error state)

## Quest Markers and Zone Triggers

> Source: `decompiled/project/ZoneAnnounce.cs` lines 26-27, 69-81

Zones can auto-complete or auto-assign quests on entry:
- `CompleteQuestOnEnter`: Quest completed when entering zone
- `CompleteSecondQuestOnEnter`: Second quest completed on zone entry
- `AssignQuestOnEnter`: Quest assigned when entering zone

## Guild Quests

> Source: `decompiled/project/GuildManager.cs` lines 45-48

Guild quests are a separate system with:
- `QuestAskItem` lines: SimPlayers asking about quest items
- `QuestAskEnding` lines: Ending the quest conversation
- `GuildQuestObjective`: The current item being sought
- `PossibleGuildQuestObjectives`: Pool of items that can be guild quest targets

## Items as Quest Sources

> Source: `decompiled/project/Item.cs` lines 154-156

Items can directly assign or complete quests when read/used:
- `Quest AssignQuestOnRead`: Reading this item starts a quest
- `Quest CompleteOnRead`: Reading this item completes a quest
