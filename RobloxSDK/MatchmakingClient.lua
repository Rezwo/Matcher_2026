--!strict

local HttpService = game:GetService("HttpService")
local MessagingService = game:GetService("MessagingService")
local TeleportService = game:GetService("TeleportService")
local Players = game:GetService("Players")

local Types = require(script.Parent.Types)

type PlayerId = Types.PlayerId
type TicketId = Types.TicketId
type Region = Types.Region
type MatchmakingRating = Types.MatchmakingRating
type PartyMember = Types.PartyMember
type Party = Types.Party
type MatchmakingResult = Types.MatchmakingResult
type TicketStatus = Types.TicketStatus
type Match = Types.Match
type MatchCreatedCallback = Types.MatchCreatedCallback
type ClientConfiguration = Types.ClientConfiguration
type GameMode = Types.GameMode

type MatchmakingClientInstance = {
	Configuration: ClientConfiguration,
	OnMatchCreated: MatchCreatedCallback?,
	IsListening: boolean,
	ActiveTickets: { [PlayerId]: TicketId },
}

local DefaultRetryAttempts: number = 3
local DefaultRetryDelaySeconds: number = 1
local DefaultRequestTimeoutSeconds: number = 30

local MatchmakingClient = {}
MatchmakingClient.__index = MatchmakingClient

function MatchmakingClient.New(Configuration: ClientConfiguration): MatchmakingClientInstance
	assert(Configuration.ServerUrl, "ServerUrl is required")
	assert(Configuration.AuthKey, "AuthKey is required")
	assert(Configuration.Topic, "Topic is required")
	assert(Configuration.PlaceId, "PlaceId is required")

	local NormalizedUrl: string = Configuration.ServerUrl
	if string.sub(NormalizedUrl, -1) == "/" then
		NormalizedUrl = string.sub(NormalizedUrl, 1, -2)
	end

	local Instance: MatchmakingClientInstance = setmetatable({
		Configuration = {
			ServerUrl = NormalizedUrl,
			AuthKey = Configuration.AuthKey,
			Topic = Configuration.Topic,
			PlaceId = Configuration.PlaceId,
			AutoTeleport = if Configuration.AutoTeleport ~= nil then Configuration.AutoTeleport else true,
			RetryAttempts = Configuration.RetryAttempts or DefaultRetryAttempts,
			RetryDelaySeconds = Configuration.RetryDelaySeconds or DefaultRetryDelaySeconds,
			RequestTimeoutSeconds = Configuration.RequestTimeoutSeconds or DefaultRequestTimeoutSeconds,
		},
		OnMatchCreated = nil,
		IsListening = false,
		ActiveTickets = {},
	}, MatchmakingClient) :: any

	return Instance
end

function MatchmakingClient.CreateParty(
	Self: MatchmakingClientInstance,
	LeaderId: PlayerId,
	LeaderName: string,
	LeaderRating: MatchmakingRating,
	PreferredRegion: Region
): Party
	local NewMember: PartyMember = {
		PlayerId = LeaderId,
		PlayerName = LeaderName,
		MatchmakingRating = LeaderRating,
	}

	return {
		Members = { NewMember },
		AverageMatchmakingRating = LeaderRating,
		PreferredRegion = PreferredRegion,
	}
end

function MatchmakingClient.AddMemberToParty(
	Self: MatchmakingClientInstance,
	TargetParty: Party,
	NewPlayerId: PlayerId,
	NewPlayerName: string,
	NewPlayerRating: MatchmakingRating
): Party
	local NewMember: PartyMember = {
		PlayerId = NewPlayerId,
		PlayerName = NewPlayerName,
		MatchmakingRating = NewPlayerRating,
	}

	local UpdatedMembers: { PartyMember } = table.clone(TargetParty.Members)
	table.insert(UpdatedMembers, NewMember)

	local TotalRating: number = 0
	local MemberCount: number = #UpdatedMembers
	for Index = 1, MemberCount do
		TotalRating += UpdatedMembers[Index].MatchmakingRating
	end

	return {
		Members = UpdatedMembers,
		AverageMatchmakingRating = TotalRating / MemberCount,
		PreferredRegion = TargetParty.PreferredRegion,
	}
end

function MatchmakingClient.RemoveMemberFromParty(
	Self: MatchmakingClientInstance,
	TargetParty: Party,
	TargetPlayerId: PlayerId
): Party?
	local UpdatedMembers: { PartyMember } = {}
	local MemberCount: number = #TargetParty.Members

	for Index = 1, MemberCount do
		local CurrentMember: PartyMember = TargetParty.Members[Index]
		if CurrentMember.PlayerId ~= TargetPlayerId then
			table.insert(UpdatedMembers, CurrentMember)
		end
	end

	local NewMemberCount: number = #UpdatedMembers
	if NewMemberCount == 0 then
		return nil
	end

	local TotalRating: number = 0
	for Index = 1, NewMemberCount do
		TotalRating += UpdatedMembers[Index].MatchmakingRating
	end

	return {
		Members = UpdatedMembers,
		AverageMatchmakingRating = TotalRating / NewMemberCount,
		PreferredRegion = TargetParty.PreferredRegion,
	}
end

function MatchmakingClient._MakeRequest(
	Self: MatchmakingClientInstance,
	Method: string,
	Endpoint: string,
	Body: { [string]: any }?
): (boolean, any)
	local Url: string = Self.Configuration.ServerUrl .. Endpoint
	local Attempts: number = Self.Configuration.RetryAttempts or DefaultRetryAttempts

	for Attempt = 1, Attempts do
		local Success: boolean, Response: any = pcall(function()
			local RequestOptions: { [string]: any } = {
				Url = Url,
				Method = Method,
				Headers = {
					["Content-Type"] = "application/json",
					["X-Api-Key"] = Self.Configuration.AuthKey,
				},
			}

			if Body then
				RequestOptions.Body = HttpService:JSONEncode(Body)
			end

			return HttpService:RequestAsync(RequestOptions)
		end)

		if Success and Response.Success then
			local DecodeSuccess: boolean, DecodedBody: any = pcall(function()
				return HttpService:JSONDecode(Response.Body)
			end)
			return true, if DecodeSuccess then DecodedBody else Response.Body
		end

		if Attempt < Attempts then
			task.wait(Self.Configuration.RetryDelaySeconds or DefaultRetryDelaySeconds)
		else
			local ErrorMessage: string = if Success
				then `HTTP {Response.StatusCode}: {Response.Body}`
				else tostring(Response)
			return false, ErrorMessage
		end
	end

	return false, "Request failed after all retry attempts"
end

function MatchmakingClient.SubmitMatchmakingTicket(
	Self: MatchmakingClientInstance,
	TargetParty: Party,
	TargetGameMode: GameMode?
): MatchmakingResult
	local RequestBody: { [string]: any } = {
		Members = TargetParty.Members,
		PreferredRegion = TargetParty.PreferredRegion,
	}

	if TargetGameMode then
		RequestBody.GameMode = TargetGameMode
	end

	local Success: boolean, Response: any = Self:_MakeRequest("POST", "/submit", RequestBody)

	if not Success then
		return {
			Success = false,
			ErrorMessage = `Failed to submit ticket: {Response}`,
		}
	end

	local TicketId: TicketId = if type(Response) == "string"
		then string.gsub(Response, '"', '')
		else Response

	if #TargetParty.Members == 1 then
		Self.ActiveTickets[TargetParty.Members[1].PlayerId] = TicketId
	end

	return {
		Success = true,
		TicketId = TicketId,
	}
end

function MatchmakingClient.CancelMatchmakingTicket(
	Self: MatchmakingClientInstance,
	TargetTicketId: TicketId
): boolean
	local Success: boolean, _Response: any = Self:_MakeRequest("DELETE", `/ticket/{TargetTicketId}`, nil)

	if Success then
		for PlayerId, StoredTicketId in Self.ActiveTickets do
			if StoredTicketId == TargetTicketId then
				Self.ActiveTickets[PlayerId] = nil
				break
			end
		end
	end

	return Success
end

function MatchmakingClient.GetTicketStatus(
	Self: MatchmakingClientInstance,
	TargetTicketId: TicketId
): TicketStatus?
	local Success: boolean, Response: any = Self:_MakeRequest("GET", `/ticket/{TargetTicketId}`, nil)

	if not Success then
		return nil
	end

	return Response :: TicketStatus
end

function MatchmakingClient.CancelPlayerTicket(
	Self: MatchmakingClientInstance,
	TargetPlayerId: PlayerId
): boolean
	local TicketId: TicketId? = Self.ActiveTickets[TargetPlayerId]
	if not TicketId then
		return false
	end

	return Self:CancelMatchmakingTicket(TicketId)
end

function MatchmakingClient._HandleMatchFound(
	Self: MatchmakingClientInstance,
	MessageData: { [string]: any }
): ()
	local DecodeSuccess: boolean, MatchData: any = pcall(function()
		return HttpService:JSONDecode(MessageData.Data)
	end)

	if not DecodeSuccess then
		warn("[MatchmakingClient] Failed to decode match data")
		return
	end

	local ReceivedMatch: Match = {
		MatchId = MatchData.MatchId,
		PlayerIds = MatchData.PlayerIds,
	}

	for _, PlayerId: PlayerId in ReceivedMatch.PlayerIds do
		Self.ActiveTickets[PlayerId] = nil
	end

	if Self.OnMatchCreated then
		task.spawn(Self.OnMatchCreated, ReceivedMatch)
	end

	if Self.Configuration.AutoTeleport then
		Self:_TeleportMatchedPlayers(ReceivedMatch)
	end
end

function MatchmakingClient._TeleportMatchedPlayers(
	Self: MatchmakingClientInstance,
	TargetMatch: Match
): ()
	local PlayersToTeleport: { Player } = {}

	for _, PlayerId: PlayerId in TargetMatch.PlayerIds do
		local FoundPlayer: Player? = Players:GetPlayerByUserId(PlayerId)
		if FoundPlayer then
			table.insert(PlayersToTeleport, FoundPlayer)
		end
	end

	if #PlayersToTeleport == 0 then
		return
	end

	local TeleportOptions: TeleportOptions = Instance.new("TeleportOptions")
	TeleportOptions.ShouldReserveServer = true

	local Success: boolean, ErrorMessage: string? = pcall(function()
		TeleportService:TeleportAsync(Self.Configuration.PlaceId, PlayersToTeleport, TeleportOptions)
	end)

	if not Success then
		warn(`[MatchmakingClient] Teleport failed: {ErrorMessage}`)
	end
end

function MatchmakingClient.StartListening(Self: MatchmakingClientInstance): boolean
	if Self.IsListening then
		warn("[MatchmakingClient] Already listening for matches")
		return false
	end

	local Success: boolean, ErrorMessage: string? = pcall(function()
		MessagingService:SubscribeAsync(Self.Configuration.Topic, function(Message)
			Self:_HandleMatchFound(Message)
		end)
	end)

	if Success then
		Self.IsListening = true
		print(`[MatchmakingClient] Listening on topic: {Self.Configuration.Topic}`)
	else
		warn(`[MatchmakingClient] Failed to subscribe: {ErrorMessage}`)
	end

	return Success
end

function MatchmakingClient.QueuePlayer(
	Self: MatchmakingClientInstance,
	TargetPlayer: Player,
	Rating: MatchmakingRating,
	PreferredRegion: Region?,
	TargetGameMode: GameMode?
): MatchmakingResult
	local Party: Party = Self:CreateParty(
		TargetPlayer.UserId,
		TargetPlayer.Name,
		Rating,
		PreferredRegion or "NorthAmerica"
	)

	return Self:SubmitMatchmakingTicket(Party, TargetGameMode)
end

MatchmakingClient.GameModes = {
	Solo = { Name = "1v1", MinPlayers = 2, MaxPlayers = 2 },
	Duos = { Name = "2v2", MinPlayers = 4, MaxPlayers = 4 },
	Squads = { Name = "4v4", MinPlayers = 8, MaxPlayers = 8 },
	FreeForAll = { Name = "FreeForAll", MinPlayers = 1, MaxPlayers = 12 },
}

function MatchmakingClient.DequeuePlayer(
	Self: MatchmakingClientInstance,
	TargetPlayer: Player
): boolean
	return Self:CancelPlayerTicket(TargetPlayer.UserId)
end

return MatchmakingClient
