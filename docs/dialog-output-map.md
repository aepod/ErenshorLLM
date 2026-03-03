# Erenshor Dialog Output Map

Complete map of every game dialog output point that produces canned SimPlayer/NPC text which can be routed through the `/v1/paraphrase` endpoint.

## Architecture Overview

### Output Sinks (how text reaches the player)

| Sink | Method | Color | Purpose |
|------|--------|-------|---------|
| Main Chat | `UpdateSocialLog.LogAdd(string)` | white/default | Say channel |
| Main Chat (colored) | `UpdateSocialLog.LogAdd(string, string)` | varies | Shout, group, guild, whisper |
| Local Chat | `UpdateSocialLog.LocalLogAdd(string)` | white | Nearby NPC speech |
| Combat Log | `UpdateSocialLog.CombatLogAdd(string)` | - | Damage numbers (not dialog) |
| Group Queue | `SimPlayerGrouping.AddStringForDisplay(string, string)` | `#00B2B7` | Queued group chat |
| Shout Queue | `SimPlayerShoutParse.QueueShout` (private List) | `#FF9000` | Queued shout channel |
| Say Queue | `SimPlayerShoutParse.QueueSay` (private List) | white | Queued say channel |
| Whisper Queue | `SimPlayerMngr.QueueResponse` (private string) | `#FF62D1` | Queued whisper |
| Whisper List | `SimPlayerMngr.Responses` (List\<WhisperData\>) | `#FF62D1` | Multi-whisper queue |

### Color-to-Channel Map

| Color Code | Channel |
|------------|---------|
| (none/white) | Say |
| `#FF9000` | Shout |
| `#00B2B7` | Group |
| `green` | Guild |
| `#FF62D1` | Whisper (output color) |
| `#EF0BAC` | Whisper (SimTradeWindow variant) |
| `yellow` | System messages |

### PersonalizeString

`SimPlayerMngr.PersonalizeString(string, SimPlayer)` is the core text transform. It handles:
- Third-person speech substitution (I'm -> Name is, etc.) for `TypesInThirdPerson` sims
- Typo/shorthand generation
- Applied BEFORE output to chat in most cases

The `NN` placeholder in dialog lists is replaced with `GameData.PlayerStats.MyName` before PersonalizeString runs.

### Key Hooking Strategy

There are TWO primary funnel points for intercepting dialog:

1. **`UpdateSocialLog.LogAdd(string, string)`** -- All colored chat eventually passes through here. A Prefix patch can intercept and paraphrase any string before it enters the log.

2. **`SimPlayerGrouping.AddStringForDisplay(string, string)`** -- All group chat passes through this queue. A Prefix patch here catches group text before it enters the delayed output queue.

However, **hooking at the funnel loses SimPlayer context**. The formatted string already contains the sim name and format prefix (e.g., "Name tells the group: text"). To get the SimPlayer reference for personality-driven paraphrasing, we need to hook **upstream** at the point where the text is generated.

---

## Group Chat (SimPlayerGrouping)

### Group: Join Greeting
- **Hook**: `SimPlayerGrouping.InviteToGroup` (public, instance)
- **Trigger**: Player invites a SimPlayer to group
- **Canned text**: Random from `Hellos` list (e.g., "Hey all!", "What's up team!")
- **Text source**: `Hellos[Random.Range(0, Hellos.Count)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `simPlayer` local variable (the sim being invited)
- **PersonalizeString**: Yes, applied at output time
- **Constraints**: Text is wrapped in `AddStringForDisplay()`. Same pattern repeated 4x (once per group slot). Also outputs a hardcoded "You're a lot higher than me..." message when level diff > 3.
- **Paraphrase trigger**: `group_join`

### Group: Dismiss Goodbye (Alive)
- **Hook**: `SimPlayerGrouping.DismissMember1` through `DismissMember4` (public, instance)
- **Trigger**: Player kicks a sim from group while sim is alive
- **Canned text**: Random from `Goodbyes` list
- **Text source**: `Goodbyes[Random.Range(0, Goodbyes.Count)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `myAvatar` (= `GameData.GroupMembers[N].MyAvatar`)
- **PersonalizeString**: Yes
- **Constraints**: Uses `UpdateSocialLog.LogAdd` directly, not `AddStringForDisplay`. 4 separate methods (DismissMember1-4) with identical logic.
- **Paraphrase trigger**: `group_leave`

### Group: Dismiss Angry (Dead)
- **Hook**: `SimPlayerGrouping.DismissMember1` through `DismissMember4` (public, instance)
- **Trigger**: Player kicks a sim from group while sim is dead
- **Canned text**: Random from `Angry` list
- **Text source**: `Angry[Random.Range(0, Angry.Count)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `myAvatar`
- **PersonalizeString**: Yes
- **Constraints**: Also decrements `OpinionOfPlayer` and sets `ignorePlayer`.
- **Paraphrase trigger**: `group_leave_angry`

### Group: Command Acknowledge (Attack/Follow/Guard/Pull/Caution/Aggro)
- **Hook**: `SimPlayerGrouping.GroupAttack`, `GroupFollow`, `GroupGuard`, `GroupPull`, `GroupCaution`, `GroupAggro`, `HoldPulls` (all public, instance)
- **Trigger**: Player issues a group command
- **Canned text**: Random from `Affirms` list (e.g., "Roger!", "On it!")
- **Text source**: `Affirms[Random.Range(0, Affirms.Count)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `simPlayerTracking.MyAvatar`
- **PersonalizeString**: Yes
- **Constraints**: Called for each group member. Uses `AddStringForDisplay`.
- **Paraphrase trigger**: `group_ack`

### Group: Lost/Far Away
- **Hook**: `SimPlayerGrouping.GroupResetMovementDebug` (public, instance)
- **Trigger**: Sim detects it's far from player (>30 units)
- **Canned text**: Random from `Lost` list
- **Text source**: `Lost[Random.Range(0, Lost.Count)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `simPlayerTracking.MyAvatar`
- **PersonalizeString**: Yes
- **Constraints**: Only fires ~30% of the time when distance > 30.
- **Paraphrase trigger**: `group_lost`

### Group: Mana Check
- **Hook**: `SimPlayerGrouping.CallOutMana` (public, instance)
- **Trigger**: Player requests mana status
- **Canned text**: `"{N}% mana"` -- computed from current/max mana
- **Text source**: Hardcoded format string with calculated percentage
- **Format**: `"Name tells the group: {N}% mana"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GameData.GroupMembers[N]`
- **PersonalizeString**: No
- **Constraints**: Pure data output. Could paraphrase but value is informational.
- **Paraphrase trigger**: `group_mana` (low priority)

### Group: Location Report
- **Hook**: `SimPlayerGrouping.ReportLoc` (public, instance)
- **Trigger**: Player asks where group members are
- **Canned text**: `"I'm at X, Y, Z"` or `"I'm right next to you"`
- **Text source**: Hardcoded with position data, or hardcoded string
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `simPlayerTracking.MyAvatar`
- **PersonalizeString**: Yes
- **Constraints**: Location data should be preserved in paraphrase.
- **Paraphrase trigger**: `group_location`

### Group: Invis Cast Request
- **Hook**: `SimPlayerGrouping.InvisGroup` (public, instance)
- **Trigger**: Player requests group invis
- **Canned text**: `"Stay close, casting invis..."` or `"I don't think anyone can cast that yet."`
- **Text source**: Hardcoded strings
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GameData.GroupMembers[0].MyAvatar`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_invis`

### Group: Pull Refusal
- **Hook**: `SimPlayerGrouping.GroupPull` (public, instance)
- **Trigger**: Player orders pull with invalid target or no puller assigned
- **Canned text**: `"Uh... you're our puller I thought?"` or `"I can't pull that target..."`
- **Text source**: Hardcoded strings
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GameData.GroupMembers[0].MyAvatar`
- **PersonalizeString**: Yes (for first variant)
- **Paraphrase trigger**: `group_pull_refuse`

### Group: Main Assist Announcement
- **Hook**: `SimPlayerGrouping.EnsureMainAssist` (public, instance)
- **Trigger**: MA reassignment (death, group change)
- **Canned text**: `"Main assist will be me again, assist me!"`
- **Text source**: Hardcoded string
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `DesignatedMA.MyAvatar`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_ma_assign`

---

## Group Chat from NPC.cs (Combat Callouts)

### Group: Healing Callouts
- **Hook**: `NPC.SimHealAIUpdate` (private, instance -- within NPC's healing logic)
- **Trigger**: Sim casts a heal or buff on group member
- **Canned text**: `"Casting HEALNAME on TargetName"`, `"HOT INCOMING on SPELLNAME on TargetName"`
- **Text source**: Hardcoded format with spell/target names
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim` (NPC.ThisSim -> SimPlayer reference)
- **PersonalizeString**: Yes
- **Constraints**: Many healing callout variants at lines 1330, 1361, 1587, 1637, 1669, 1735, 1770, 1808, 1835, 1878, 1907, 1938 in NPC.cs. All follow the same pattern.
- **Paraphrase trigger**: `group_heal_callout`

### Group: Buffing Callouts
- **Hook**: `NPC.SimBuffAIUpdate` (private, instance -- buff casting logic in NPC)
- **Trigger**: Sim casts a buff spell on group member
- **Canned text**: `"SPELLNAME incoming on TargetName"`
- **Text source**: Hardcoded format
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_buff_callout`

### Group: Targeting Callout
- **Hook**: Multiple locations in `NPC` (lines 2893, 2936)
- **Trigger**: Sim acquires a combat target
- **Canned text**: Random from `SimPlayerGrouping.Targeting` list + mob name
- **Text source**: `GameData.SimPlayerGrouping.Targeting[Random.Range(...)] + " " + tar.NPCName`
- **Format**: `"Name tells the group: {targeting text} MobName"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Constraints**: Uses `AddStringForDisplay`, not direct LogAdd.
- **Paraphrase trigger**: `group_target`

### Group: Assist Callout
- **Hook**: Multiple locations in `NPC` (lines 4176-4230)
- **Trigger**: Sim assists the main assist
- **Canned text**: ~10 hardcoded variants like `"assisting Name on MobName"`, `"I'm on MobName!"`, `"Killing MobName!"`
- **Text source**: Hardcoded strings with target name interpolation
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Constraints**: 10+ variants selected by `Random.Range(0, 10)`. Heavy target name interpolation.
- **Paraphrase trigger**: `group_assist`

### Group: Taunt Callout
- **Hook**: Multiple locations in `NPC` (lines 4257, 4289, 4314)
- **Trigger**: Sim taunts a mob
- **Canned text**: `"taunting MobName!"`, `"AE Taunting! Heals on me!"`, `"taunting MobName, stay on your current target!"`
- **Text source**: Hardcoded strings
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_taunt`

### Group: CC Callout
- **Hook**: `NPC` (line 3986)
- **Trigger**: Sim casts crowd control
- **Canned text**: `"casting CCSPELL on MobName"`
- **Text source**: Hardcoded format
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_cc`

### Group: CC Immune Warning
- **Hook**: `NPC` (line 3913)
- **Trigger**: Sim detects a CC-immune mob
- **Canned text**: `"PLAYERNAME! MobName can't be stunned, get on that one ASAP!"`
- **Text source**: Hardcoded format
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_cc_immune`

### Group: Snare/Fear Callout
- **Hook**: `NPC` (lines 4345, 4357, 4384)
- **Trigger**: Sim casts snare or fear
- **Canned text**: `"casting SPELL on MobName"`, `"SPELL incoming on MobName, get ready to chase it!"`
- **Text source**: Hardcoded format with spell/mob names
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_snare`

### Group: OOM / Medding
- **Hook**: `NPC` (line 1994)
- **Trigger**: Sim runs out of mana
- **Canned text**: `"OOM! Casting meditate!"`
- **Text source**: Hardcoded string
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_oom`

### Group: Mana Restore
- **Hook**: `NPC` (line 4851)
- **Trigger**: Sim uses mana regen ability
- **Canned text**: `"Restoring my mana, hold on!"`
- **Text source**: Hardcoded string
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_mana_regen`

### Group: Environmental Damage
- **Hook**: `NPC` (line 4414)
- **Trigger**: Sim takes environmental damage
- **Canned text**: Random from `SimPlayerLanguage.EnvDmg` list (e.g., "OW OW OW")
- **Text source**: `GameData.SimLang.GetEnvDmg()`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_env_dmg`

### Group: Stance Switch
- **Hook**: `Stats` (line 3540)
- **Trigger**: Sim switches combat stance
- **Canned text**: `"Switching to STANCENAME stance!"`
- **Text source**: Hardcoded format with stance name
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `Myself.MyNPC.ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_stance` (low priority)

---

## Group Chat from SimPlayer.cs

### Group: Death Reaction
- **Hook**: `SimPlayer` (lines 599, 603, 607) -- inside private death handling
- **Trigger**: Group member dies or all group members die
- **Canned text**: Random from `MyDialog.Died` list, or `"It's telling me you're all dead! I can't revive!"`
- **Text source**: `MyDialog.Died[Random.Range(...)]` or hardcoded
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `this` (the SimPlayer instance)
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_death`

### Group: Close Call
- **Hook**: `SimPlayer` (line 580)
- **Trigger**: Sim narrowly survives combat
- **Canned text**: `"Close one!"`
- **Text source**: Hardcoded string
- **Format**: `"Name tells the group: Close one!"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `this`
- **PersonalizeString**: No
- **Paraphrase trigger**: `group_close_call`

### Group: Aggro Alert
- **Hook**: `SimPlayer` (line 392)
- **Trigger**: Sim gets aggro while grouped
- **Canned text**: `"I have aggro!, trying to get to you!"`
- **Text source**: Hardcoded string
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#FF62D1`) -- goes through `LoadResponse`
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_aggro_alert`

### Group: Mana/Health Wait (Puller)
- **Hook**: `SimPlayer` (lines 1582-1609) -- in pull logic
- **Trigger**: Puller waiting on mana/health before pulling
- **Canned text**: `"need mana..."`, `"medding up for a sec..."`, `"I'm OOM hang on..."`, `"Waiting on a heal"`, `"gotta heal up."`, `"need some life before I pull more."`, `"waiting on group mana..."`
- **Text source**: Hardcoded strings
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_wait_resources`

### Group: Pulling Callout
- **Hook**: `SimPlayer` (line 1655)
- **Trigger**: Sim begins pulling a mob
- **Canned text**: `"pulling MobName"`
- **Text source**: Hardcoded format with `PullTarget.transform.name`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_pull`

### Group: Pull Arrived
- **Hook**: `SimPlayer` (line 1754)
- **Trigger**: Pulled mob arrives at camp
- **Canned text**: `"MobName is here, attack it!"`
- **Text source**: Hardcoded format
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_pull_arrived`

### Group: Zoning Out
- **Hook**: `SimPlayer` (lines 1905, 1917)
- **Trigger**: Sim zones out (flee) or can't find exit
- **Canned text**: `"Zoning out!"` or `"There's nowhere to go!"`
- **Text source**: Hardcoded strings
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_zone`

### Group: Aggro Gain (Character.cs)
- **Hook**: `Character.OnAggroGain` (line 1258 in Character.cs)
- **Trigger**: Sim gains aggro from a mob
- **Canned text**: Random from `MyDialog.GetAggro()` (e.g., "it's on me")
- **Text source**: `SimPlayer.MyDialog.GetAggro()`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GetComponent<SimPlayer>()`
- **PersonalizeString**: Yes
- **Constraints**: Uses `AddStringForDisplay`.
- **Paraphrase trigger**: `group_aggro`

### Group: Death Down
- **Hook**: `Character` (line 909)
- **Trigger**: Sim dies in group
- **Canned text**: `"I'm down!"`
- **Text source**: Hardcoded string
- **Format**: `"Name tells the group: I'm down!"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `MyNPC.ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_down`

### Group: XP Loss on Respawn
- **Hook**: `Respawn` (line 99)
- **Trigger**: Grouped sim respawns after death
- **Canned text**: Random from `SimPlayerMngr.XPLossMsg` list
- **Text source**: `GameData.SimMngr.XPLossMsg[Random.Range(...)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `simPlayerTracking.MyAvatar`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_xploss`

---

## Loot Dialog (ItemIcon.cs)

### Group: Loot Request
- **Hook**: `ItemIcon.SimLootRolls` (around lines 1554-1598)
- **Trigger**: A group member sees loot they want
- **Canned text**: Random from `SimPlayerLanguage.WantsDrop` list (via `GetLootReq()`)
- **Text source**: `MyDialog.GetLootReq()` -> `WantsDrop[Random.Range(...)]`
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GameData.GroupMembers[N].MyAvatar`
- **PersonalizeString**: Yes
- **Constraints**: Uses `AddStringForDisplay`. Checked for each of 4 group slots.
- **Paraphrase trigger**: `group_loot_request`

### Group: Loot Guild Quest Match
- **Hook**: `ItemIcon` (lines 1559, 1572, 1585, 1598)
- **Trigger**: Loot matches a sim's guild quest objective
- **Canned text**: `"Oh! That's the item I mentioned I was after in guild chat!"`
- **Text source**: Hardcoded string
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GameData.GroupMembers[N].MyAvatar`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `group_loot_quest`

### Group: Loot Impressed (Whisper)
- **Hook**: `SimPlayer` (line 4201)
- **Trigger**: Sim is impressed by a loot drop
- **Canned text**: `GetImpressed() + " " + itemName + " " + GetImpressedEnd()`
- **Text source**: `MyDialog.GetImpressed()`, `MyDialog.GetImpressedEnd()`
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#FF62D1`) -- via `LoadResponse`
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `loot_impressed`

---

## Say Channel (SimPlayerShoutParse)

### Say: Greeting Response
- **Hook**: `SimPlayerShoutParse.RespondToGreetingSay` (private, instance)
- **Trigger**: Player says a greeting in /say
- **Canned text**: `GetGreeting() + " " + GetTargetedHello(simTracking)` -- combination greeting
- **Text source**: `MyDialog.GetGreeting()` + `MyDialog.GetTargetedHello()`
- **Format**: `"Name says: {text}"`
- **Channel**: say (default white, via QueueSay)
- **SimPlayer access**: `simPlayer` (loop variable from `ActiveSimInstances`)
- **PersonalizeString**: Yes (NN replaced first, then PersonalizeString)
- **Constraints**: Private method. Only responds to sims within 10 units. Must not be in group.
- **Paraphrase trigger**: `say_greeting`

### Say: Goodnight Response
- **Hook**: `SimPlayerShoutParse.RespondToGoodnightSay` (private, instance)
- **Trigger**: Player says goodnight in /say
- **Canned text**: `GetGoodnight()`
- **Text source**: `MyDialog.GetGoodnight()`
- **Format**: `"Name says: {text}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes (NN replaced, then PersonalizeString)
- **Paraphrase trigger**: `say_goodnight`

### Say: LFG Response (Accept)
- **Hook**: `SimPlayerShoutParse.RespondToLfgSay` (public, instance)
- **Trigger**: Player says LFG in /say
- **Canned text**: `GetOTW()` (e.g., "coming")
- **Text source**: `MyDialog.GetOTW()`
- **Format**: `"Name says: {text}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_lfg_accept`

### Say: LFG Response (Decline)
- **Hook**: `SimPlayerShoutParse.RespondToLfgSay` (public, instance)
- **Trigger**: Player says LFG but sim declines
- **Canned text**: `GetDeclineGroup()`
- **Text source**: `MyDialog.GetDeclineGroup()`
- **Format**: `"Name says: {text}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_lfg_decline`

### Say: Level Up Congratulation
- **Hook**: `SimPlayerShoutParse.RespondToLvlUpSay` (private, instance)
- **Trigger**: Player says a ding/level-up message in /say
- **Canned text**: Random from `SimPlayerMngr.LevelUpCongratulations` list
- **Text source**: `GameData.SimMngr.LevelUpCongratulations[Random.Range(...)]`
- **Format**: `"Name says: {text}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Constraints**: Troublemaker > 6 pulls from higher indices (snarky responses).
- **Paraphrase trigger**: `say_grats`

### Say: Insult Response
- **Hook**: Inside `RespondToGreetingSay`, `RespondToGoodnightSay`, `RespondToLfgSay` (when `IsThisNotMeSpecifically` is true)
- **Trigger**: Player insults a specific sim by name
- **Canned text**: `"PlayerName " + GetInsult()`
- **Text source**: `SimPlayerLanguage.GetInsult()`
- **Format**: `"Name says: {PlayerName} {insult}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_insult`

### Say: Info Response (Item/NPC/Quest)
- **Hook**: `SimPlayerShoutParse.ShareInfoItemSay`, `ShareInfoNPCSay`, `ShareInfoQuestSay` (all private, instance)
- **Trigger**: Player asks about items/NPCs/quests in /say
- **Canned text**: Answers from `KnowledgeDatabase.GetItemDropAnswer()`, `GetNPCAnswer()`, `GetQuestAnswer()`, `CheckTheWiki()`, `GetIDKGeneric()`
- **Text source**: KnowledgeDatabase methods
- **Format**: `"Name says: {text}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Constraints**: Proxmity check (10 units). Coroutine-based -- `DirectSaySearch` is an IEnumerator.
- **Paraphrase trigger**: `say_knowledge`

### Say: Invis Offer
- **Hook**: `SimPlayerShoutParse.FindInvisCaster` (private, instance)
- **Trigger**: Player requests invis in /say or /shout
- **Canned text**: `"I can do it for you, hang on"`
- **Text source**: Hardcoded string
- **Format**: `"Name says: {text}"`
- **Channel**: say (via QueueSay)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_invis_offer`

### Say: Slot Request Response
- **Hook**: `SimPlayerShoutParse.RespondToSlotRequest` (private, instance) -> delegates to `SimPlayerMngr.DoSlotRequest`
- **Trigger**: Player asks about equipment ("where did you get...")
- **Canned text**: Various equipment info
- **Text source**: Dynamic equipment info from `DoSlotRequest` coroutine
- **Format**: `"Name says: {text}"`
- **Channel**: say
- **SimPlayer access**: via `whichSim` SimPlayerTracking
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_slot_request`

---

## Shout Channel (SimPlayerShoutParse)

### Shout: Greeting Response
- **Hook**: `SimPlayerShoutParse.RespondToGreeting` (private, instance)
- **Trigger**: Player shouts a greeting
- **Canned text**: `GetGreeting()` (the generic one, no targeted hello)
- **Text source**: `MyDialog.GetGreeting()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Constraints**: Also queues a whisper with `GetGreeting() + GetTargetedHello()` for known, liked sims. Private method.
- **Paraphrase trigger**: `shout_greeting`

### Shout: Greeting Whisper (from shout)
- **Hook**: `SimPlayerShoutParse.RespondToGreeting` (private, instance)
- **Trigger**: Player shouts a greeting and sim knows + likes the player
- **Canned text**: `GetGreeting() + " " + GetTargetedHello(simTracking)` -- the full memory-driven hello
- **Text source**: `MyDialog.GetGreeting()` + `MyDialog.GetTargetedHello()`
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (via QueueWhisper -- but NOTE: whispers in QueueWhisper are NOT output in Update!)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Constraints**: QueueWhisper list appears to NOT be consumed in Update() (see lines 51-57 -- timer resets but no LogAdd). These whispers go to `Responses` list instead.
- **Paraphrase trigger**: `whisper_greeting`

### Shout: Goodnight Response
- **Hook**: `SimPlayerShoutParse.RespondToGoodnight` (private, instance)
- **Trigger**: Player shouts goodnight
- **Canned text**: `GetGoodnight()`
- **Text source**: `MyDialog.GetGoodnight()`
- **Format**: `"Name says: {text}"` (NOTE: says, not shouts -- likely a design choice for intimacy)
- **Channel**: shout channel color (`#FF9000`, via QueueShout, but format says "says:")
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_goodnight`

### Shout: LFG Accept
- **Hook**: `SimPlayerShoutParse.RespondToLfg` (public, instance)
- **Trigger**: Player shouts LFG
- **Canned text**: `GetOTW()`
- **Text source**: `MyDialog.GetOTW()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_lfg_accept`

### Shout: LFG Decline
- **Hook**: `SimPlayerShoutParse.RespondToLfg` (public, instance)
- **Trigger**: Player shouts LFG but sim declines
- **Canned text**: `GetDeclineGroup()`
- **Text source**: `MyDialog.GetDeclineGroup()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_lfg_decline`

### Shout: Level Up Congratulation
- **Hook**: `SimPlayerShoutParse.RespondToLvlUp` (private, instance)
- **Trigger**: Player shouts a ding/level-up message
- **Canned text**: `GetLevelUpCelebration()`
- **Text source**: `MyDialog.GetLevelUpCelebration()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_grats`

### Shout: Obscenity Response (Exclamation)
- **Hook**: `SimPlayerShoutParse.RespondToObscene` (private, instance)
- **Trigger**: Player uses obscenity in chat
- **Canned text**: `GetExclamation()`
- **Text source**: `MyDialog.GetExclamation()`
- **Format**: `"Name says: {text}"` (via QueueShout, so actually shout color)
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_exclaim`

### Shout: Obscenity Insult
- **Hook**: `SimPlayerShoutParse.RespondToObscene` (private, instance)
- **Trigger**: Player uses obscenity targeting a specific sim
- **Canned text**: `"PlayerName " + GetInsult()`
- **Text source**: `SimPlayerLanguage.GetInsult()`
- **Format**: `"Name shouts: {PlayerName} {insult}"`
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_insult`

### Shout: Info Response (Item/NPC/Quest)
- **Hook**: `SimPlayerShoutParse.ShareInfoItemInShout`, `ShareInfoNPCInShout`, `ShareInfoQuestInShout` (public/private, instance)
- **Trigger**: Player asks about items/NPCs/quests in /shout
- **Canned text**: From `KnowledgeDatabase` methods
- **Text source**: `GetItemDropAnswer()`, `GetNPCAnswer()`, `GetQuestAnswer()`, `CheckTheWiki()`, `GetIDKGeneric()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`, via QueueShout)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_knowledge`

---

## Ambient Shout (SimPlayerMngr)

### Shout: Small Talk (Zone Comment)
- **Hook**: `SimPlayerMngr.SimSmallTalk` (private, instance)
- **Trigger**: Random ambient sim banter (timer-based)
- **Canned text**: From `SimPlayerLanguage.GetGeneric()` (-> `ZoneComments`) or `LiveGuildData.RecruitmentStrings`
- **Text source**: `GetGeneric()` -> `GameData.CurrentZoneAnnounce.ZoneComments[...]` or `guildDataByID.RecruitmentStrings[...]`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `simPlayer` (from `FindSimInstance(sim)`)
- **PersonalizeString**: Yes
- **Constraints**: Private method. Gated by `GameData.SimBanter` flag and `banterDel` timer.
- **Paraphrase trigger**: `ambient_smalltalk`

### Shout: Sim-to-Sim Insult Banter
- **Hook**: `SimPlayerMngr.SimSmallTalk` (private, instance)
- **Trigger**: Random ambient banter -- sim insults another sim
- **Canned text**: Greeting + other sim's name + `GetExclamation()` + `GetInsult()`
- **Text source**: `SimPlayerLanguage.GetExclamation()`, `SimPlayerLanguage.GetInsult()`
- **Format**: `"Name shouts: {greeting} {otherSimName} {exclamation} {insult}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Constraints**: Triggers `NeedToRetort` queue for the insulted sim to respond.
- **Paraphrase trigger**: `ambient_insult`

### Shout: Generic Greeting
- **Hook**: `SimPlayerMngr.SimShoutGreeting` (private, instance)
- **Trigger**: Random ambient greeting (timer-based)
- **Canned text**: `GetGeneric()` (zone comments) or compound greeting from `GetGreeting()` + `GetExclamation()`
- **Text source**: `MyDialog.GetGeneric()` or `GameData.SimLang.GetGreeting()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `simPlayer`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `ambient_greeting`

### Shout: Retort / Exclamation / Confirm / Negative
- **Hook**: `SimPlayerMngr.Update` (private, instance -- lines 1709-1750)
- **Trigger**: Queued retort or answer to a question from another sim
- **Canned text**: `GetConfirm()`, `GetNegative()`, `GetExclamation()`, `GetRetort()`
- **Text source**: `MyDialog.GetConfirm()`, `MyDialog.GetNegative()`, `MyDialog.GetExclamation()`, `MyDialog.GetRetort()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `simPlayer2/3/4/5` (from `FindActivePlayerByName`)
- **PersonalizeString**: Yes
- **Constraints**: Private Update method. Hard to hook individual outputs -- would need to hook the LogAdd call or the FindActivePlayerByName.
- **Paraphrase trigger**: `ambient_retort`

### Shout: Sim Level Up (Congrats from others)
- **Hook**: `SimPlayerMngr.Update` (private, instance -- lines 1699-1707)
- **Trigger**: A sim levels up, other sims congratulate
- **Canned text**: `GetLevelUpCelebration()`
- **Text source**: `MyDialog.GetLevelUpCelebration()`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `simPlayer` (from `ActiveSimInstances[Random]`)
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `ambient_grats`

---

## SimPlayer Ambient (SimPlayer.cs)

### Shout: WTB (Want to Buy)
- **Hook**: `SimPlayer` (line 469) -- in private Update/behavior loop
- **Trigger**: Sim wants to buy an item
- **Canned text**: `"WTB ItemName, offering N gold. Open a trade with me."`
- **Text source**: Hardcoded format with item data
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `ambient_wtb`

### Shout: Random Exclamation
- **Hook**: `SimPlayer` (line 479) -- in private Update/behavior loop
- **Trigger**: Random exclamation during behavior
- **Canned text**: `GetExclamation() + "!"`
- **Text source**: `MyDialog.GetExclamation()`
- **Format**: `"Name shouts: {text}!"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `ambient_exclaim`

### Shout: LFG Call
- **Hook**: `SimPlayer` (lines 1136, 1284) -- in POI/behavior logic
- **Trigger**: Sim looking for group
- **Canned text**: Random from `MyDialog.LFGPublic` list + area name + "you can lead."
- **Text source**: `MyDialog.LFGPublic[Random.Range(...)]`
- **Format**: `"Name shouts: {lfg text} AreaName you can lead."`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `ambient_lfg`

### Shout: TRAIN Warning
- **Hook**: `SimPlayer` (line 1366) -- in flee logic
- **Trigger**: Sim flees with a mob chasing (train)
- **Canned text**: `"TRAIN!!! MOBNAME TO ZONE!!"`
- **Text source**: Hardcoded format
- **Format**: `"Name shouts: TRAIN!!! MOBNAME TO ZONE!!"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `ambient_train`

### Say: Hail NPC
- **Hook**: `SimPlayer` (lines 1124, 1272) -- in POI interaction
- **Trigger**: Sim hails an NPC
- **Canned text**: `"Hail, NPCName"` and NPC responds `"mutters something inaudible to SimName"`
- **Text source**: Hardcoded format
- **Format**: `"Name says: Hail, NPCName"` + `"NPCName mutters something inaudible to Name"`
- **Channel**: say (white)
- **SimPlayer access**: `this`
- **PersonalizeString**: No
- **Constraints**: The NPC response is hardcoded. Both lines are direct LogAdd.
- **Paraphrase trigger**: `say_hail` (low priority)

### Say: Arriving at Group
- **Hook**: `SimPlayer` (line 754)
- **Trigger**: Sim arrives to join player's group
- **Canned text**: `"I'm here, joining your group now."`
- **Text source**: Hardcoded string
- **Format**: `"Name says: {text}"`
- **Channel**: say (white)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_arriving`

### Say: Lag Out
- **Hook**: `NPC` (line 5162) -- debug/recovery
- **Trigger**: Sim resets after getting stuck
- **Canned text**: `"I lagged out for a sec... what were we doing again?"`
- **Text source**: Hardcoded string
- **Format**: `"Name says: {text}"`
- **Channel**: say (white)
- **SimPlayer access**: `ThisSim`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `say_lag` (low priority)

### Shout: Sim Level Up Announcement
- **Hook**: `Stats.DoLevelUp` (line 788 in Stats.cs)
- **Trigger**: A sim levels up
- **Canned text**: Random from `SimPlayerMngr.LevelUpCelebrations` list
- **Text source**: `GameData.SimMngr.LevelUpCelebrations[Random.Range(...)]`
- **Format**: `"Name shouts: {text}"`
- **Channel**: shout (`#FF9000`)
- **SimPlayer access**: `component` (SimPlayer component)
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `shout_levelup`

---

## Whisper Channel (SimPlayerMngr)

All whisper output goes through `SimPlayerMngr.QueueResponse` (private string) and `SimPlayerMngr.Responses` (List\<WhisperData\>), which are consumed in `SimPlayerMngr.Update()` and output via `UpdateSocialLog.LogAdd(msg, "#FF62D1")`.

### Whisper: Greeting (Proactive)
- **Hook**: `SimPlayerMngr.GreetPlayer` (private, instance)
- **Trigger**: Sim proactively greets player (knows player, opinion > 6, cooldown elapsed)
- **Canned text**: `GetTargetedHello(simTracking)` -- full memory-driven hello (alt detection, been-a-while, etc.)
- **Text source**: `SimPlayerLanguage.GetTargetedHello()` -> `HelloBuilder()`
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#FF62D1`)
- **SimPlayer access**: `FindSimPlayer(sim)`
- **PersonalizeString**: Yes
- **Constraints**: Private method. This is the main personality-driven hello that references past sessions.
- **Paraphrase trigger**: `whisper_greeting`

### Whisper: Guild Invite Offer
- **Hook**: `SimPlayerMngr.InviteToJoinGuild` (private, instance)
- **Trigger**: Sim offers player a guild invite
- **Canned text**: `"Hey NN, have you ever thought about a guild? I bet I could get you into GuildName..."``
- **Text source**: Hardcoded format
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#FF62D1`)
- **SimPlayer access**: `FindSimPlayer(sim)`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `whisper_guild_invite`

### Whisper: Group Invite
- **Hook**: `SimPlayerMngr.SimInvite` (private, instance)
- **Trigger**: Sim invites player to group
- **Canned text**: `GetInvite() + " " + ZoneTerm + " " + GetJustification()`
- **Text source**: `SimPlayerLanguage.GetInvite()`, `GetJustification()`
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#FF62D1`)
- **SimPlayer access**: `simPlayer` (from `FindSimPlayer(sim)`)
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `whisper_group_invite`

### Whisper: Respond to Player Whisper (SimRespondToSay)
- **Hook**: `SimPlayerMngr.SimRespondToSay` (public, instance)
- **Trigger**: Player whispers a sim -- massive switch of response types
- **Canned text**: Many categories:
  - Greeting: `GetReturnGreeting()`
  - Status: `"I'm in ZoneName"` + `GetReturnGreeting()`
  - Level: `"I'm level N."`
  - LFG accept: `GetOTW()`, `"Add me to your party!"`
  - LFG decline: `GetDeclineGroup()`, `"sorry, I'm already in a group."`
  - Already in group: `"I'm already in your group!"`
  - Zone request: `"I'm coming, hold on!"` or `"I can't get to that place..."`
  - Obscenity: `GetAnger()`
  - Apology: from `ApologyResponses` list
  - Gratitude: `GetAcknowledgeGratitude()`
  - BeenAWhile: from `SimPlayerLanguage.BeenAWhile` list
  - Guild membership: `"I'm not in a guild!"`, `"Yeah! You've always been nice..."`, etc.
  - "What's up": `"Hey, I'm just hanging in a group right now."` or `"Just hanging out solo..."`
  - Didn't understand: from `DidNotUnderstand` list
  - Confirm/Deny: `GetConfirm()`, `GetDenials()`
  - Item/NPC/Quest info: delegated to knowledge database
- **Text source**: Multiple `SimPlayerLanguage.Get*()` methods, hardcoded strings, `DidNotUnderstand` list
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#FF62D1`)
- **SimPlayer access**: `simPlayer` (from `FindSimPlayer(whichSim)`)
- **PersonalizeString**: Yes
- **Constraints**: This is a massive method (~800 lines) with deep branching. Each branch sets `QueueResponse` then adds to `Responses` list. Many branches include a `BeenAWhile` check when the sim hasn't seen the player recently.
- **Paraphrase trigger**: `whisper_response` (with sub-types)

### Whisper: Arriving in Zone
- **Hook**: `SimPlayerMngr` (lines 1521, 1533) -- in zone arrival logic
- **Trigger**: Sim arrives in player's zone after being invited
- **Canned text**: `"I'm here, heading your way."`
- **Text source**: Hardcoded string
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper
- **SimPlayer access**: `FindSimPlayer(sim)`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `whisper_arriving`

### Whisper: GM Warning (Obscenity)
- **Hook**: `SimPlayerMngr` (line 2202)
- **Trigger**: Player uses obscenities repeatedly
- **Canned text**: From `GMWarningsObscenities` list (escalating warnings)
- **Text source**: `GMWarningsObscenities[warnings - 1]`
- **Format**: `"[WHISPER FROM] GM-Burgee: {text}"`
- **Channel**: whisper
- **SimPlayer access**: None (GM character)
- **PersonalizeString**: No
- **Constraints**: NOT sim dialog. System message. Do not paraphrase.
- **Paraphrase trigger**: SKIP (system message)

### Whisper: World Event Response
- **Hook**: `SimPlayerMngr.QueueWhisperFromExternal` (public, instance) -- called from world event code
- **Trigger**: Guild world event completion or rival response
- **Canned text**: From `CongratsForWorldEvent` or `FriendsClubResponseToWorldEvent` lists
- **Text source**: `CongratsForWorldEvent[Random.Range(...)]` or `FriendsClubResponseToWorldEvent[Random.Range(...)]`
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper
- **SimPlayer access**: via the `SimPlayerTracking` responder
- **PersonalizeString**: Yes (applied inside `QueueWhisperFromExternal`)
- **Paraphrase trigger**: `whisper_world_event`

---

## Guild Channel (GuildManager)

### Guild: Sim-Initiated Chat (Topic-Based)
- **Hook**: `GuildManager.SimPlayerChatInput` (public, instance)
- **Trigger**: Timer-based ambient guild chat from a sim
- **Canned text**: From `GuildTopic.SimPlayerActivations` list, or compound item/NPC/level questions
- **Text source**: `GuildTopic.SimPlayerActivations[Random.Range(...)]` or dynamically constructed questions
- **Format**: `"Name tells the guild: {text}"`
- **Channel**: guild (`green`)
- **SimPlayer access**: `simPlayer` (from `GameData.SimMngr.FindSimPlayer(simPlayerTracking)`)
- **PersonalizeString**: Yes
- **Constraints**: The initiating message is output directly, then responses are queued via `ParseGuildChatInput` or `RespondToKnownTopic`.
- **Paraphrase trigger**: `guild_chat_initiate`

### Guild: Topic Response
- **Hook**: `GuildManager.PostResponseToGuildChat` (private, IEnumerator)
- **Trigger**: Responses to guild chat topics
- **Canned text**: From `GuildTopic.Responses` list (+ optional `Preceed`/`End` wrapping)
- **Text source**: `topic.Responses[Random.Range(...)]`
- **Format**: `"Name tells the guild: {text}"`
- **Channel**: guild (`green`)
- **SimPlayer access**: `GameData.SimMngr.FindSimPlayer(_responders[i])` -- available at the output point
- **PersonalizeString**: Yes
- **Constraints**: Coroutine with `WaitForSeconds(0.5-15.5f)` between responses. The primary output method for guild responses.
- **Paraphrase trigger**: `guild_chat_response`

### Guild: Personalized Topic Response
- **Hook**: `GuildManager.PostPersonalizedResponseToGuildChat` (private, IEnumerator)
- **Trigger**: Responses to greetings/goodnights/dings in guild
- **Canned text**: From sim-specific `SimPlayerLanguage` lists (Greetings, Goodnight, LevelUpCelebration)
- **Text source**: `component.Greetings[...]`, `component.Goodnight[...]`, `component.LevelUpCelebration[...]`
- **Format**: `"Name tells the guild: {text}"`
- **Channel**: guild (`green`)
- **SimPlayer access**: `_simActual` parameter (SimPlayer)
- **PersonalizeString**: Yes
- **Constraints**: Coroutine.
- **Paraphrase trigger**: `guild_chat_personal`

### Guild: Player-Initiated Chat Response
- **Hook**: `GuildManager.ParseGuildChatInput` (public, IEnumerator)
- **Trigger**: Player types in guild chat, sims respond
- **Canned text**: Topic responses, knowledge database answers, greetings, goodnights, etc.
- **Text source**: Multiple sources depending on what player said
- **Format**: `"Name tells the guild: {text}"`
- **Channel**: guild (`green`)
- **SimPlayer access**: Available through responder tracking
- **PersonalizeString**: Yes
- **Constraints**: IEnumerator (coroutine). Complex branching. Also handles NPC/Item/Quest lookups.
- **Paraphrase trigger**: `guild_chat_player_response`

### Guild: Knowledge Response (Item/NPC/Quest)
- **Hook**: `GuildManager.ShareInfoItemInGuild`, `ShareInfoNPCInGuild`, `ShareInfoQuestInGuild` (public, instance)
- **Trigger**: Guild member asks about items/NPCs/quests
- **Canned text**: From `KnowledgeDatabase` methods
- **Text source**: Same as shout/say knowledge but routed through `PostResponseToGuildChat`
- **Format**: `"Name tells the guild: {text}"`
- **Channel**: guild (`green`)
- **SimPlayer access**: Through `GetResponders()` list
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `guild_knowledge`

### Guild: Admin Messages
- **Trigger**: Sim joins/leaves guild, world events
- **Examples**: `"[GUILD ADMIN MSG]: A new player has joined GUILDNAME! Welcome SimName!"`
- **Paraphrase trigger**: SKIP (system messages)

---

## Trade/Item Dialog

### Say: Item Accept/Decline (SimPlayer.cs)
- **Hook**: `SimPlayer.GiveItem` / `SimPlayer.AcceptItem` (lines 2526-2666)
- **Trigger**: Player gives item to sim
- **Canned text**: Multiple variants:
  - `"I can't use that with my current setup."`
  - `"Thank you! I'll put my old item away in case I need it later!"`
  - `"Thanks! I really needed this."`
  - `"Ok, I'll use this for now."`
  - `"Ah, I can't use this right now but thanks anyhow"`
  - `"Oops, here's your ItemName back."`
  - `"My primary weapon is two handed, so I can't equip this right now."`
  - `"Oops, I have a better one of these."`
  - `"Ok, I'll switch that item out."`
  - `"Ok, I'll equip that."`
- **Text source**: Hardcoded strings
- **Format**: `"Name says: {text}"`
- **Channel**: say (white)
- **SimPlayer access**: `this`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `trade_item_response`

### Say: Trade Window Responses (SimTradeWindow.cs)
- **Hook**: `SimTradeWindow` (multiple methods)
- **Trigger**: Trade window interactions
- **Canned text**:
  - `"Thanks! I owe you N gold, here it is."`
  - `"Wow! Thank you for that! We can put this to use I bet."`
  - `"Oops, here's your ItemName back."`
- **Text source**: Hardcoded strings
- **Format**: `"Name says: {text}"`
- **Channel**: say (white)
- **SimPlayer access**: `parent.GetComponent<SimPlayer>()`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `trade_response`

### Whisper: Trade Gratitude (SimTradeWindow.cs)
- **Hook**: `SimTradeWindow` (lines 164-233)
- **Trigger**: Player gives spell/item through trade
- **Canned text**: `GetGratitude()`, `"Thanks! I didn't have this one yet!"`, `"I don't need that at the moment..."`, `"I've got this one already..."`
- **Text source**: `GameData.SimLang.GetGratitude()` + hardcoded strings
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (`#EF0BAC` -- different color than normal whispers!)
- **SimPlayer access**: `parent.GetComponent<SimPlayer>()`
- **PersonalizeString**: Yes
- **Constraints**: Uses `#EF0BAC` color (pink/magenta) instead of normal whisper color `#FF62D1`.
- **Paraphrase trigger**: `trade_gratitude`

### Say: Inspect Responses (SimInspect.cs)
- **Hook**: `SimInspect` (lines 480-562)
- **Trigger**: Player tries to upgrade sim gear at forge
- **Canned text**: `"That's an Aura! We can't upgrade that."`, `"I don't have enough Sivakruxes!"`, `"Yes! Thank you for helping me!"`, `"Awww that stinks!"`, `"Don't we need to do this at a forge?"`
- **Text source**: Hardcoded strings
- **Format**: `"Name tells the group: {text}"`
- **Channel**: group (`#00B2B7`)
- **SimPlayer access**: `GameData.InspectSim.Who`
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `inspect_response`

### Say: Inspect Tampering (SimItemDisplay.cs)
- **Hook**: `SimItemDisplay` (line 247)
- **Trigger**: Player messes with sim's equipped items
- **Canned text**: `"Stop messing with my stuff!"`
- **Text source**: Hardcoded string
- **Format**: `"Name says: Stop messing with my stuff!"`
- **Channel**: say (white)
- **SimPlayer access**: `GameData.InspectSim.Who`
- **PersonalizeString**: No
- **Paraphrase trigger**: `inspect_tamper`

---

## NPC Aggro Area (NPCAggroArea.cs)

### Say/Whisper: Proximity Greeting
- **Hook**: `NPCAggroArea.OnTriggerEnter` (lines 49-155)
- **Trigger**: Player enters sim's aggro area (proximity)
- **Canned text**: Greeting from sim language, or invis offer `"Still need that invis? Here you go."`
- **Text source**: `GetGreeting()` or hardcoded
- **Format**: `"Name says: {text}"`
- **Channel**: say (white)
- **SimPlayer access**: `component3` (SimPlayer from collider)
- **PersonalizeString**: Yes
- **Paraphrase trigger**: `proximity_greeting`

### Whisper: Rare Item Reaction (Molorai Mask)
- **Hook**: `NPCAggroArea.OnTriggerEnter` (lines 68-155)
- **Trigger**: Sim notices player wearing Eldoth/Molorai mask
- **Canned text**: 25+ hardcoded reactions about the mask (e.g., "no way thats the Eldoth mask??", "dude how did you get the Molorai mask lol")
- **Text source**: Hardcoded strings (not from lists)
- **Format**: `"[WHISPER FROM] Name: {text}"`
- **Channel**: whisper (via `LoadResponse`)
- **SimPlayer access**: `component3` (SimPlayer)
- **PersonalizeString**: Yes
- **Constraints**: 25 unique hardcoded strings, selected by `Random.Range(0, 25)`. These are special -- they reference specific in-game items and should be paraphrased carefully.
- **Paraphrase trigger**: `proximity_rare_item`

### Say: NPC Aggro Message
- **Hook**: `NPC` (line 2150)
- **Trigger**: NPC aggro text (non-sim NPCs)
- **Canned text**: From `AggroMsg` field
- **Format**: `"Name says: {AggroMsg}"`
- **Channel**: say (white)
- **SimPlayer access**: N/A (this is actual NPC, not SimPlayer)
- **PersonalizeString**: No
- **Constraints**: Not SimPlayer dialog -- actual NPC aggro text. Skip paraphrasing.
- **Paraphrase trigger**: SKIP (actual NPC)

---

## Boss Events

### FernallaFightEvent
- **Trigger**: Fallen Fernalla boss encounter
- **Canned text**: Hardcoded boss shouts (e.g., "I warned you not to approach me, child!")
- **Channel**: shout (`#FF9000`)
- **Paraphrase trigger**: SKIP (boss scripted dialog)

### FernallaPortalShouts
- **Trigger**: Fernalla portal zone
- **Canned text**: Random from `Shouts` list + hardcoded
- **Channel**: shout (`#FF9000`)
- **Paraphrase trigger**: SKIP (boss scripted dialog)

### WaveEvent
- **Trigger**: Wave-based encounter
- **Canned text**: `IntroText`, `End`, random from `ShoutBetweenWaves`, `BossAlert`
- **Channel**: shout (`#FF9000`)
- **Paraphrase trigger**: SKIP (boss scripted dialog)

### LighthouseHealBox
- **Trigger**: Kio the Darkbringer encounter
- **Canned text**: Random from `KioShouts` + hardcoded
- **Channel**: shout (`#FF9000`)
- **Paraphrase trigger**: SKIP (boss scripted dialog)

### NPCShoutListener
- **Trigger**: NPC proximity shout
- **Canned text**: Random from `Responses` list
- **Channel**: shout (`#FF9000`)
- **Paraphrase trigger**: SKIP (static NPC, not sim)

---

## NPC Dialog (NPCDialogManager)

### NPC Quest/Vendor Dialog
- **Hook**: `NPCDialogManager` (lines 101, 127)
- **Trigger**: Player interacts with NPC
- **Canned text**: `ReturnString` (computed dialog response)
- **Format**: `"Name says: {ReturnString}"`
- **Channel**: say (white) + local log
- **SimPlayer access**: N/A (actual NPCs, not SimPlayers)
- **PersonalizeString**: No
- **Paraphrase trigger**: SKIP (actual NPC dialog -- not sim)

---

## Vendor Dialog (GameData.cs)

### Vendor Buy/Sell
- **Hook**: `GameData` (lines 638-658)
- **Trigger**: Player interacts with vendor
- **Canned text**: `"That'll be N gold for the ItemName"`, `"I'll give you N gold for the ItemName"`, `"Sorry, but I'm not buying..."`
- **Format**: `"VendorName says: {text}"`
- **Channel**: say (white) + local log
- **SimPlayer access**: N/A (vendor NPC)
- **PersonalizeString**: No
- **Paraphrase trigger**: SKIP (vendor NPC)

---

## Trade Window (TradeWindow.cs)

### NPC Trade Dialog
- **Hook**: `TradeWindow` (lines 271-342)
- **Trigger**: Crafting/trade with NPC vendors
- **Canned text**: `DialogOnSuccess`, `DisableText`, fallback text
- **Format**: `"Name says: {text}"`
- **Channel**: say (white) + local log
- **SimPlayer access**: N/A (NPC vendors)
- **PersonalizeString**: No
- **Paraphrase trigger**: SKIP (NPC vendor)

---

## Charmed NPC (CharmedNPC.cs)

### Charmed Pet Dialog
- **Hook**: `CharmedNPC` (lines 120, 130, 144)
- **Trigger**: Player commands charmed NPC
- **Canned text**: `"I can't do that..."`, `"backing off..."`, `"As you wish, master."`
- **Format**: `"Name says: {text}"`
- **Channel**: say (white/yellow)
- **SimPlayer access**: N/A (charmed NPC, not sim)
- **PersonalizeString**: No
- **Paraphrase trigger**: SKIP (charmed NPC)

---

## GM/System Messages

### GM Exploit Warning
- **Hook**: `PlayerControl` (lines 2438-2456)
- **Trigger**: Player detected abusing NPC pathing
- **Canned text**: Escalating GM warnings
- **Format**: `"[WHISPER FROM] GM-Burgee: {text}"`
- **Channel**: whisper (`yellow`)
- **Paraphrase trigger**: SKIP (system message)

---

## Optimal Hook Points Summary

### Tier 1: Highest Value (personality-driven, frequent)
| Priority | Hook Target | Channel | Frequency | Text Source |
|----------|------------|---------|-----------|-------------|
| 1 | `SimPlayerMngr.GreetPlayer` | whisper | Medium | `HelloBuilder()` -- memory-driven |
| 2 | `SimPlayerMngr.SimRespondToSay` | whisper | High | Multiple `Get*()` methods |
| 3 | `SimPlayerMngr.SimSmallTalk` | shout | High | `GetGeneric()`, insults |
| 4 | `SimPlayerMngr.SimShoutGreeting` | shout | High | `GetGeneric()`, `GetGreeting()` |
| 5 | `SimPlayerGrouping.InviteToGroup` | group | Medium | `Hellos` list |
| 6 | `SimPlayerGrouping.DismissMember1-4` | group | Low | `Goodbyes`/`Angry` lists |
| 7 | `GuildManager.PostResponseToGuildChat` | guild | Medium | `GuildTopic.Responses` |
| 8 | `GuildManager.PostPersonalizedResponseToGuildChat` | guild | Medium | Sim language lists |

### Tier 2: Medium Value (combat callouts, responses)
| Priority | Hook Target | Channel | Frequency | Text Source |
|----------|------------|---------|-----------|-------------|
| 9 | `SimPlayerShoutParse.RespondToGreetingSay` | say | Low | `GetGreeting()` + `GetTargetedHello()` |
| 10 | `SimPlayerShoutParse.RespondToLfg` | shout | Low | `GetOTW()`/`GetDeclineGroup()` |
| 11 | `SimPlayerMngr.SimInvite` | whisper | Low | `GetInvite()` + `GetJustification()` |
| 12 | `SimPlayerShoutParse.RespondToObscene` | shout | Low | `GetExclamation()`/`GetInsult()` |
| 13 | NPC healing/buff callouts | group | High | Hardcoded spell names |
| 14 | NPC assist/taunt callouts | group | High | Hardcoded combat text |

### Tier 3: Lower Value (hardcoded, functional)
| Priority | Hook Target | Channel | Notes |
|----------|------------|---------|-------|
| 15 | `SimPlayer` death text | group | `MyDialog.Died` list |
| 16 | `SimPlayer` WTB/LFG/TRAIN | shout | Hardcoded formats |
| 17 | Item accept/decline | say | Hardcoded equipment responses |
| 18 | Knowledge responses | all | KnowledgeDatabase answers |
| 19 | Trade gratitude | whisper | Limited variation |

### SKIP: Do Not Paraphrase
- Boss encounter scripted dialog (Fernalla, Kio, WaveEvent)
- Actual NPC quest/vendor dialog
- Charmed NPC responses
- GM-Burgee system warnings
- Guild admin messages
- Vendor buy/sell text

---

## Key Architectural Notes

### Output Funnels

1. **`UpdateSocialLog.LogAdd(string, string)`** -- ALL chat with color passes through this static method. A single Harmony Prefix here can intercept everything, but you lose SimPlayer context (must parse the name from the formatted string).

2. **`SimPlayerGrouping.AddStringForDisplay(string, string)`** -- Group chat funnel. Buffers text into `Disp` list, output via `DispTxt()` in FixedUpdate. Hook here to intercept group chat before delay.

3. **`SimPlayerMngr.Responses` list / `QueueResponse`** -- Whisper funnel. All whispers queue here and output in `Update()` via `LogAdd(msg, "#FF62D1")`. Hook at `LoadResponse` or `QueueWhisperFromExternal` to intercept whispers.

4. **`SimPlayerShoutParse.QueueShout`/`QueueSay` (private lists)** -- Shout/say funnel. Pre-formatted strings queue here and output in `Update()`. Access requires reflection (private).

### PersonalizeString Timing

In most cases, `PersonalizeString` is applied BEFORE the text enters the queue. This means:
- The text reaching the output funnel is ALREADY personalized (third person, typos, etc.)
- Paraphrasing should happen BEFORE PersonalizeString, or the paraphrase endpoint must handle already-personalized text
- Ideal hook point is at the generation site, BEFORE PersonalizeString is called

### SimPlayer Access Pattern

To get personality data for the paraphrase endpoint, you need:
- `SimPlayer` instance -- for `MyDialog` (SimPlayerLanguage), `Troublemaker`, `TypoChance`
- `SimPlayerTracking` -- for `SimName`, `Level`, `ClassName`, `KnowsPlayer`, `OpinionOfPlayer`, `GuildID`, `Rival`
- `SimPlayerTracking` is at `GameData.SimMngr.Sims[simPlayer.myIndex]`

### Private Method Hooking

Many key methods are private. Harmony can still patch these:
```csharp
var method = AccessTools.Method(typeof(ClassName), "PrivateMethodName");
harmony.Patch(method, prefix: new HarmonyMethod(typeof(MyPatch), nameof(Prefix)));
```

### Coroutine Considerations

`PostResponseToGuildChat`, `PostPersonalizedResponseToGuildChat`, and `ParseGuildChatInput` are IEnumerator coroutines. Harmony patching coroutines requires patching the MoveNext method of the compiler-generated state machine class, which is more complex. Consider hooking at the `UpdateSocialLog.LogAdd` call inside the coroutine instead, or hooking the calling method.
