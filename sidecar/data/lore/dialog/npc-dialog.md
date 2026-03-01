# NPC Dialog and SimPlayer Language

> Extracted from decompiled Erenshor source code.

---

## SimPlayer Chat System

> Source: `decompiled/project/SimPlayerLanguage.cs`
> Note: The actual dialog lines for each category are stored as serialized List<string> on a
> MonoBehaviour in the Unity scene. The C# code only defines the lists and fallback defaults.
> The fallback defaults (used when lists are empty) are shown below.

SimPlayers use a language system (`SimPlayerLanguage`) that selects random lines from
categorized lists. The system uses "NN" as a placeholder token that gets replaced with
the player's character name via regex.

### Dialog Categories and Fallback Defaults

| Category | Fallback Text | Purpose |
|---|---|---|
| Greetings | "Hi" | Initial greeting |
| ReturnGreeting | "Hi" | Response to player greeting (uses NN token) |
| Invites | "Come xp" | Group invitation (uses NN token) |
| Justifications | "it'll be good" | Reason to group |
| Confirms | "roger" | Confirmation |
| GenericLines | (uses ZoneComments) | Zone-specific small talk |
| Aggro | "it's on me" | Alerting about aggro |
| Died | "dang dang dang!" | Death reaction |
| InsultsFun | "You're stinky and your mother dresses you funny" | Playful insults |
| RetortsFun | "back at you haha take that" | Responses to insults |
| Exclamations | "Oh my GOSH" | Surprised reactions |
| Denials | "no" | Denial |
| DeclineGroup | "busy atm" | Declining group invite |
| Negative | "busy atm" | Negative response |
| LFGPublic | "." | Looking for group (public) |
| OTW | "coming" | On the way |
| Goodnight | "bye" | Logging off (uses NN token) |
| Hello | "heya" | General hello (uses NN token) |
| LocalFriendHello | "Hiya NN" | Greeting a known friend |
| UnsureResponse | "uh what?" | Confused response |
| AngerResponse | ":(" | Angry response |
| AcknowledgeGratitude | ":)" | Thanking response |
| Affirms | "Yeah" | Affirmation |
| EnvDmg | "OW OW OW" | Environmental damage reaction |
| WantsDrop | (uses global SimLang) | Requesting loot |
| Gratitude | "Yes! Thanks! " | Thanking for loot |
| Impressed | "nice" | Impressed reaction |
| ImpressedEnd | "" | End of impressed sequence |
| LevelUpCelebration | "" | Celebrating a level up |

### Alt Character Recognition Dialog

> Source: `decompiled/project/SimPlayerLanguage.cs` lines 363-394

When a SimPlayer recognizes the player is on an alt character (different from the one
they grouped with before), they randomly select from these hardcoded lines:

1. "You're on an alt?? Jump over onto {PreviousCharName} and let's go!"
2. "Hey get on {PreviousCharName} so we can group!"
3. "New character?? Why aren't you on {PreviousCharName} so we can xp?"
4. "Where's {PreviousCharName}? I wanna group again!"
5. "What are you... making alts now? Get on {PreviousCharName}!"
6. "I'm gonna need you go get on {PreviousCharName} so we can group again."
7. "Hi! You gonna be playing {PreviousCharName} today? I wanna group!"
8. "Who's got time for alts? You should be on {PreviousCharName}!"
9. "I need to make an alt too so I can group with you when you're not playing {PreviousCharName}..."
10. "Uh... {CurrentCharName} is the toon you're gonna be on today? Lame!"

### Memory-Based Greeting System

> Source: `decompiled/project/SimPlayerLanguage.cs` lines 345-438 (HelloBuilder method)

SimPlayers build compound greetings using their memory of previous interactions:

- **Been Away**: If the player hasn't logged in for 3+ days, they use `BeenAWhile` lines.
- **Return to Zone**: If they grouped in a specific zone recently, they mention it: "{greeting}! {ReturnToZone line} {ZoneName}!"
- **Good Last Outing**: If the previous session had good XP and few deaths, they comment positively.
- **Bad Last Outing**: If the previous session had many deaths or poor XP gain, they comment negatively.
- **Got an Item**: If the player got a notable item, the SimPlayer mentions it: "{GotAnItemLastOuting line} {ItemName}."

---

## NPC Dialog System

> Source: `decompiled/project/NPCDialog.cs` and `decompiled/project/NPCDialogManager.cs`

### Architecture

NPC dialog is handled by two classes:
- `NPCDialog` (MonoBehaviour): Holds individual dialog entries with optional keyword triggers
- `NPCDialogManager` (MonoBehaviour): Manages the NPC's dialog options and routes player input

Each NPC can have multiple `NPCDialog` components. Dialog text is stored in the Unity
`Dialog` field (TextArea) on each component -- these are serialized in scene data, not in code.

### Dialog Flow

1. Player hails an NPC (or the NPC is hailed)
2. `NPCDialogManager.GenericHail()` fires for initial greeting
3. It checks class restrictions (`SpecificClass` list) -- if the player is the wrong class, it shows `Rejection` text
4. If hostile (negative faction or aggressive), the NPC says "..."
5. If in combat, the NPC says "I'm busy right now..."
6. Otherwise, it returns the default `Dialog` text with keyword highlights in green: `[keyword]`
7. If the player says a keyword, `NPCDialogManager.ParseText()` finds the matching `NPCDialog` and returns its text

### Quest Dialog

NPCDialog can trigger quest assignment or completion:
- `QuestToAssign`: Assigns a quest when dialog fires
- `QuestToComplete`: Completes a quest when dialog fires
- `GiveItem`: Gives an item to the player
- `RepeatingQuestDialog`: Shown if the quest is already done
- `RequireQuestComplete`: Only trigger this dialog option if specified quest is complete
- `KillMeOnSay`: NPC dies after speaking
- `Spawn`: Spawns a game object (enemy, etc.) after speaking

### Zone Comments

> Source: `decompiled/project/ZoneAnnounce.cs` line 14

Each zone has a `ZoneComments` list (List<string>) that SimPlayers use for zone-specific
small talk. These are set per-zone in the Unity scene data, not in code.

---

## Event Dialog (Hardcoded)

### Island Tomb / Stowaway's Step Tutorial Boss Event

> Source: `decompiled/project/StowawayPortal.cs`

During the tutorial boss encounter in the Island Tomb / Stowaway's Step:

- Line 152: "The remaining shadows reel, and gather strength from the loss of their companion" (yellow, when first skeleton dies)
- Line 169: "The remaining shadows reel, and gather strength from the loss of their companion" (yellow, when second skeleton dies)
- Line 182: "The remaining shadow becomes enraged!" (yellow, when third skeleton dies)
- Line 202: "Something approaches..." (yellow, when all four skeletons are defeated)
- Line 205: "Azynthian Keeper Shouts: You are treading in places you should not be! It is time to write an ending to your tale." (red)
- Line 132: "Azynthian Keeper Shouts: Come, my minions! Show them our strength!" (red, when Keeper summons adds)
- Line 135: "Azynthian Keeper Shouts: Minions! I need more of your lives for the cause!" (red, when Keeper is low HP and summons more adds)

### Jail Event

> Source: `decompiled/project/JailTimer.cs`

- Line 13: "You have been JAILED by staff member: GM-Burgee for breaking the rules of Erenshor."
- Line 14: "You will be automatically released in 3 minutes."
- Line 22: "You will be automatically released in 2 minutes."
- Line 27: "You will be automatically released in 1 minute."

### Death / XP Loss

> Source: `decompiled/project/Respawn.cs` line 69

- "You have died and lost some experience." (yellow)

### Login Messages

> Source: `decompiled/project/CharSelectManager.cs` lines 732-735

On character login, the following messages are displayed:

- "Loading Data..." (yellow)
- "Loading Server Data..." (yellow)
- "MESSAGE OF THE DAY: Welcome to Erenshor! Press ENTER and type /help for tips and commands" (yellow)
- "Erenshor v0.3 - 'Rising Shadows' is live! Check out the Steam announcement for highlights." (green)
- "Last login: {days} days ago." (yellow)

### Zone Entry

> Source: `decompiled/project/ZoneAnnounce.cs` line 103

- "You have entered {ZoneName} at {GameTime}"

### Faction Changes

> Source: `decompiled/project/GlobalFactionManager.cs` lines 36-40

- "You've lost standing with {FactionDesc}!" (grey)
- "You've gained standing with {FactionDesc}!" (grey)

---

## Guild Chat System

> Source: `decompiled/project/GuildManager.cs` and `decompiled/project/GuildData.cs`

Guild NPCs have conversation topics (`GuildTopic` ScriptableObjects) with:
- `SimPlayerActivations`: Lines that SimPlayers say to start a topic
- `ActivationWords`: Keywords that trigger responses
- `Responses`: Reply lines
- `RelevantScene`: Zone-specific topics
- `RequiredLevelToKnow`: Minimum level to participate
- `MaxLevelToAsk`: Maximum level that asks these questions
- `Preceed` / `End`: Lines that bookend responses

The GuildManager also has serialized lists for:
- `OutOfZoneAnswers`: Responses when asked about a zone the player hasn't been to
- `LowLevelAnswers`: Responses to low-level players
- `QuestAskItem`: Lines asking about guild quest items
- `QuestAskEnding`: Lines ending guild quest conversations
- `Signoff`: Farewell lines
- `GuildDeletedResponses`: Reactions when a guild is disbanded
- `GuildRemoveResponses`: Reactions when removed from a guild
- `ItemSearch`: Lines about searching for items
- `NPCSearch`: Lines about searching for NPCs
- `LevelAdvice`: Lines giving level advice

All actual text content is serialized in Unity scene data, not hardcoded in C# source.
