--!strict

local Types = require(script.Types)
local MatchmakingClient = require(script.MatchmakingClient)

export type PlayerId = Types.PlayerId
export type TicketId = Types.TicketId
export type MatchId = Types.MatchId
export type Region = Types.Region
export type MatchmakingRating = Types.MatchmakingRating
export type PartyMember = Types.PartyMember
export type Party = Types.Party
export type MatchmakingResult = Types.MatchmakingResult
export type TicketStatus = Types.TicketStatus
export type Match = Types.Match
export type MatchCreatedCallback = Types.MatchCreatedCallback
export type ClientConfiguration = Types.ClientConfiguration

export type MatchmakingClientInstance = typeof(MatchmakingClient.New(nil :: any))

return {
	New = MatchmakingClient.New,
	Types = Types,
}
