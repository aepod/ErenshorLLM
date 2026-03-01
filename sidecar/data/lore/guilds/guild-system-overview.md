# Guild System Overview

## Built-In Guilds

1. **Erenshor's Young** -- Default starter guild. All SimPlayers below level 10 without a guild are placed here automatically. Tutorial-focused conversations.

2. **Friends' Club** -- Antagonist "uber-guild." Only Rival SimPlayers belong here. Members are arrogant, competitive, refuse to group with outsiders. Score increases faster than other guilds.

3. **Hatty Cats** -- Template guild with cat-themed personality. Has a unique "Meow" conversation topic.

4. **Additional Template Guilds** -- The game has a `GuildTemplates` list loaded from ScriptableObject assets in Unity. The exact names of other template guilds beyond Hatty Cats are set in the Unity editor and not hardcoded in C# source. Players may encounter various auto-generated guild names.

5. **Player-Created Guilds** -- Players can create up to 4 guilds per server. These are stored in `PlayerCreatedGuilds`.

## Guild Assignment Rules

- SimPlayers below level 10 with no guild -> Erenshor's Young
- Rival SimPlayers -> Friends' Club (always, cannot leave)
- SimPlayers above level 10 -> Random chance to join a template guild (18-23 max members per guild)
- GM character (GM-Burgee) -> Excluded from all guilds
- SimPlayers in player-led guilds cannot be poached if player is guild leader (unless opinion > 25)

## Guild Effects on Dialog

### Same Guild as Player
- More trust, willingness to help
- Inside jokes and shared references
- Guild banter in guild chat channel
- May ask for guild quest help
- Casual, familiar tone

### Friends' Club Member -> Non-FC
- Arrogant, competitive, sometimes dismissive
- Will not group with outsiders
- Taunts during world events
- Higher greed values

### Friends' Club Member -> FC Member
- Camaraderie, shared superiority complex
- Coordinate for world events
- Strong group synergy (auto-form balanced parties)

### Different Guild / No Guild
- Neutral baseline interaction
- Normal friendship/opinion mechanics apply
- Standard greeting/farewell patterns
- May try to recruit player to their guild if opinion is high enough

## Guild Score and Competition

- Guilds compete for top score
- Top guild gets bonus loot drop chance
- Friends' Club gains score 60% of the time (vs 20% for others)
- World events award guild score to winning guild
- Guilds with excessively high scores may be denied additional points
