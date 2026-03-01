# Erenshor SimPlayer Personality System

This document describes how personality works in the base game code. The LLM dialog system should respect these base-game properties while extending them with the richer personality profiles in the individual SimPlayer files.

## Personality Types (from SimPlayerTracking.Personality / SimPlayer.PersonalityType)

| Value | Type | Description | Bio Source |
|-------|------|-------------|------------|
| 0 | Unassigned | Re-rolled to 1-5 with 70% chance, otherwise set to 5 | N/A |
| 1 | Nice | Friendly, helpful, positive | NiceDescriptions list |
| 2 | Tryhard | Competitive, intense, performance-focused | TryhardDescriptions list |
| 3 | Mean | Hostile, negative, troublemaker | MeanDescriptions list |
| 4 | (Unnamed) | Assigned but no specific description list | N/A |
| 5 | Neutral | Default personality, no strong traits | N/A |

Note: Personality 1 (Nice) is weighted more heavily -- if a random roll lands on values above 3, it gets re-rolled to 1 with 30% chance.

## Chat Modifiers (from SimPlayer component, set per-prefab or loaded from save)

| Flag | Effect |
|------|--------|
| TypesInAllCaps | All text converted to UPPERCASE |
| TypesInAllLowers | All text converted to lowercase |
| TypesInThirdPerson | Replaces "I/me/my" with the SimPlayer's name |
| LovesEmojis | Inserts emoticons (:) :D ;) 8) :p) after punctuation |
| TypoRate | How many words per message get typos (default 0.25-0.5) |
| TypoChance | Probability of typo per word (default 0.25) |
| RefersToSelfAs | Custom self-reference string (replaces "I/me/my") |
| SignOffLine | List of sign-off phrases appended randomly to messages |
| Abbreviates | Whether the SimPlayer uses abbreviations |

## Behavioral Attributes (from SimPlayerTracking)

| Field | Range | Effect |
|-------|-------|--------|
| LoreChase | 0-10 | How much the SimPlayer pursues lore content |
| GearChase | 0-10 | How much the SimPlayer pursues gear upgrades |
| SocialChase | 0-10 | How much the SimPlayer pursues social interaction |
| Troublemaker | 0-10 | How disruptive/antagonistic the SimPlayer is |
| DedicationLevel | 0-10 | How committed the SimPlayer is to long sessions |
| Greed | 0.0-4.0 | How greedy the SimPlayer is with loot (Rivals: 3.0-4.0) |
| OpinionOfPlayer | float | Dynamic opinion that changes based on interactions |
| Caution | bool | Whether the SimPlayer plays cautiously |
| Patience | int | How long before the SimPlayer gets bored (default 3000) |

## Special Flags

| Flag | Effect |
|------|--------|
| Rival | Belongs to Friends' Club. Aggressive. Higher greed. Will not group with player. |
| IsGMCharacter | Is GM-Burgee. Excluded from all normal SimPlayer activities. |
| KnowsPlayer | Has met the player before. Uses player's name in conversation. |
| playerFriend | Is on the player's friends list. More willing to help. |

## Bio System

SimPlayers have a BioIndex that maps to a description from one of three lists:
- NiceDescriptions (Personality 1)
- TryhardDescriptions (Personality 2)
- MeanDescriptions (Personality 3)

These lists are set in the Unity editor on the SimPlayerMngr component and are not hardcoded. The Bio text is displayed in the SimPlayer inspect window.

## Language System (SimPlayerLanguage)

Each SimPlayer has dialog pools for various situations:
- Greetings, ReturnGreeting, Hello, LocalFriendHello
- Invites, Justifications, Confirms
- GenericLines (small talk)
- Aggro, Died, InsultsFun, RetortsFun
- Exclamations, Denials, DeclineGroup, Negative
- LFGPublic, OTW (On The Way)
- Goodnight, UnsureResponse, AngerResponse
- AcknowledgeGratitude, Affirms
- EnvDmg, WantsDrop, Gratitude
- Impressed, ImpressedEnd, LevelUpCelebration
- GoodLastOuting, BadLastOuting, GotAnItemLastOuting
- ReturnToZone, BeenAWhile, Unsure

For generic SimPlayers (not prefabs), these pools are populated randomly from the SimPlayerMngr's global lists. Prefab SimPlayers may have custom dialog entries set in the Unity editor.

## How PersonalizeString Works

The PersonalizeString method applies personality transformations in order:
1. TypesInThirdPerson -> Replace I/me/my with SimPlayer's name
2. RefersToSelfAs -> Replace I/me/my with custom reference
3. SignOffLine -> Randomly append a sign-off phrase (10% chance)
4. TypoRate -> Apply random typos to words
5. TypesInAllCaps -> Convert to UPPERCASE
6. TypesInAllLowers -> Convert to lowercase
7. LovesEmojis -> Insert emoticons after punctuation

## Classes Available

| Internal Name | Display Name | Role |
|--------------|-------------|------|
| Paladin | Paladin | Tank/Healer |
| Arcanist | Arcanist | CC/DPS |
| Druid | Druid | Healer |
| Duelist | Windblade | DPS |
| Stormcaller | Stormcaller | DPS |
| Reaver | Reaver | Tank |
