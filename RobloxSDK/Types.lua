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
}

export type MatchmakingResult = {
	Success: boolean,
	TicketId: TicketId?,
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

export type Match = {
	MatchId: MatchId,
	PlayerIds: { PlayerId },
}

export type GameMode = {
	Name: string,
	MinPlayers: number,
	MaxPlayers: number,
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
}

export type HttpResponse = {
	Success: boolean,
	StatusCode: number,
	Body: string,
}

return nil
