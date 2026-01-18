--!strict

export type PlayerId = number
export type TicketId = string
export type MatchId = string
export type Region = string
export type MatchmakingRating = number

export type PartyMember = {
	PlayerId: PlayerId,
	PlayerName: string,
	MatchmakingRating: MatchmakingRating,
}

export type Party = {
	Members: { PartyMember },
	AverageMatchmakingRating: MatchmakingRating,
	PreferredRegion: Region,
	LeaderId: PlayerId, -- First member is the leader
}

export type MatchmakingResult = {
	Success: boolean,
	TicketId: TicketId?,
	ErrorMessage: string?,
}

export type CancelResult = {
	Success: boolean,
	ErrorMessage: string?,
}

export type TicketStatus = {
	TicketId: TicketId,
	Status: string,
	QueuePosition: number?,
	WaitTimeSeconds: number,
	ExpansionLevel: number,
	EstimatedWaitSeconds: number?,
}

export type TicketStatusResult = {
	Success: boolean,
	Data: TicketStatus?,
	ErrorMessage: string?,
}

export type Match = {
	MatchId: MatchId,
	PlayerIds: { PlayerId },
}

export type GameMode = {
	Name: string,
	MinPlayers: number,
	MaxPlayers: number,
	AllowBackfill: boolean?,
}

export type CustomMatchData = {
	MapPreference: string?,
	GameSettings: { [string]: any }?,
	Tags: { string }?,
}

export type SubmitOptions = {
	GameMode: GameMode?,
	Priority: number?,
	CustomData: CustomMatchData?,
}

export type MatchCreatedCallback = (Match: Match) -> ()

export type ClientConfiguration = {
	ServerUrl: string,
	AuthKey: string,
	Topic: string,
	PlaceId: number,
	AutoTeleport: boolean?,
	RetryAttempts: number?,
	RetryDelaySeconds: number?,
	RequestTimeoutSeconds: number?,
	DebugMode: boolean?,
	AutoDetectRegion: boolean?,
}

export type PartyTicketInfo = {
	TicketId: TicketId,
	LeaderId: PlayerId,
	MemberIds: { PlayerId },
}

export type HttpResponse = {
	Success: boolean,
	StatusCode: number,
	Body: string,
}

return nil
