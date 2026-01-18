--!strict

local HttpService = game:GetService("HttpService")
local MessagingService = game:GetService("MessagingService")
local TeleportService = game:GetService("TeleportService")
local Players = game:GetService("Players")
local LocalizationService = game:GetService("LocalizationService")

local Types = require(script.Parent:WaitForChild("Types"))

type PlayerId = Types.PlayerId
type TicketId = Types.TicketId
type Region = Types.Region
type MatchmakingRating = Types.MatchmakingRating
type PartyMember = Types.PartyMember
type Party = Types.Party
type MatchmakingResult = Types.MatchmakingResult
type CancelResult = Types.CancelResult
type TicketStatus = Types.TicketStatus
type TicketStatusResult = Types.TicketStatusResult
type Match = Types.Match
type MatchCreatedCallback = Types.MatchCreatedCallback
type ClientConfiguration = Types.ClientConfiguration
type GameMode = Types.GameMode
type CustomMatchData = Types.CustomMatchData
type SubmitOptions = Types.SubmitOptions
type PartyTicketInfo = Types.PartyTicketInfo

type MatchmakingClientInstance = {
	Configuration: ClientConfiguration,
	OnMatchCreated: MatchCreatedCallback?,
	IsListening: boolean,
	ActiveTickets: { [PlayerId]: TicketId },
	ActiveParties: { [TicketId]: PartyTicketInfo },
	TicketToPlayers: { [TicketId]: { PlayerId } },
	_SubscriptionConnection: RBXScriptConnection?,
	_PlayerRemovingConnection: RBXScriptConnection?,
	_RetryCount: number,
	_LastSubmitTime: number,
}

-- Constants
local DefaultRetryAttempts: number = 3
local DefaultRetryDelaySeconds: number = 1
local DefaultRequestTimeoutSeconds: number = 30
local MaxRetryAttempts: number = 5
local RetryDelaySeconds: number = 5
local MinSubmitIntervalSeconds: number = 0.5 -- Rate limiting

-- Region mapping from country codes
local RegionMapping: { [string]: string } = {
	US = "NorthAmerica", CA = "NorthAmerica", MX = "NorthAmerica",
	BR = "SouthAmerica", AR = "SouthAmerica", CL = "SouthAmerica", CO = "SouthAmerica",
	GB = "Europe", DE = "Europe", FR = "Europe", ES = "Europe", IT = "Europe", NL = "Europe", PL = "Europe",
	JP = "Asia", KR = "Asia", CN = "Asia", TW = "Asia", HK = "Asia", SG = "Asia", IN = "Asia",
	AU = "Oceania", NZ = "Oceania",
}

local MatchmakingClient = {}
MatchmakingClient.__index = MatchmakingClient

-- Preset GameModes
MatchmakingClient.GameModes = {
	Solo = { Name = "1v1", MinPlayers = 2, MaxPlayers = 2, AllowBackfill = false },
	Duos = { Name = "2v2", MinPlayers = 4, MaxPlayers = 4, AllowBackfill = false },
	Trios = { Name = "3v3", MinPlayers = 6, MaxPlayers = 6, AllowBackfill = false },
	Squads = { Name = "4v4", MinPlayers = 8, MaxPlayers = 8, AllowBackfill = false },
	FreeForAll = { Name = "FreeForAll", MinPlayers = 2, MaxPlayers = 12, AllowBackfill = true },
}

-- Preset Priority levels
MatchmakingClient.Priority = {
	Normal = 0,
	High = 50,
	Premium = 100,
	VIP = 200,
}

-- Preset Regions
MatchmakingClient.Regions = {
	NorthAmerica = "NorthAmerica",
	SouthAmerica = "SouthAmerica",
	Europe = "Europe",
	Asia = "Asia",
	Oceania = "Oceania",
}

local function Log(Self: MatchmakingClientInstance, Level: string, Message: string)
	if Self.Configuration.DebugMode or Level == "error" or Level == "warn" then
		local Prefix = `[MatchmakingClient:{Level}]`
		if Level == "error" then
			warn(Prefix, Message)
		elseif Level == "warn" then
			warn(Prefix, Message)
		else
			print(Prefix, Message)
		end
	end
end

local function ValidatePlayerId(Id: any): boolean
	return type(Id) == "number" and Id > 0 and Id == math.floor(Id)
end

local function ValidateRating(Rating: any): boolean
	return type(Rating) == "number" and Rating == Rating and Rating ~= math.huge and Rating ~= -math.huge
end

local function ValidatePartyMember(Member: any): (boolean, string?)
	if type(Member) ~= "table" then
		return false, "Member must be a table"
	end
	if not ValidatePlayerId(Member.PlayerId) then
		return false, "Invalid PlayerId"
	end
	if type(Member.PlayerName) ~= "string" or Member.PlayerName == "" then
		return false, "PlayerName must be a non-empty string"
	end
	if not ValidateRating(Member.MatchmakingRating) then
		return false, "Invalid MatchmakingRating"
	end
	return true, nil
end

local function ValidateGameMode(Mode: any): (boolean, string?)
	if type(Mode) ~= "table" then
		return false, "GameMode must be a table"
	end
	if type(Mode.Name) ~= "string" or Mode.Name == "" then
		return false, "GameMode.Name must be a non-empty string"
	end
	if type(Mode.MinPlayers) ~= "number" or Mode.MinPlayers < 1 then
		return false, "GameMode.MinPlayers must be >= 1"
	end
	if type(Mode.MaxPlayers) ~= "number" or Mode.MaxPlayers < Mode.MinPlayers then
		return false, "GameMode.MaxPlayers must be >= MinPlayers"
	end
	return true, nil
end

function MatchmakingClient.New(Configuration: ClientConfiguration): MatchmakingClientInstance
	assert(Configuration.ServerUrl and Configuration.ServerUrl ~= "", "ServerUrl is required")
	assert(Configuration.AuthKey and Configuration.AuthKey ~= "", "AuthKey is required")
	assert(Configuration.Topic and Configuration.Topic ~= "", "Topic is required")
	assert(Configuration.PlaceId and Configuration.PlaceId > 0, "PlaceId is required and must be > 0")

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
			DebugMode = Configuration.DebugMode or false,
			AutoDetectRegion = if Configuration.AutoDetectRegion ~= nil then Configuration.AutoDetectRegion else true,
		},
		OnMatchCreated = nil,
		IsListening = false,
		ActiveTickets = {},
		ActiveParties = {},
		TicketToPlayers = {},
		_SubscriptionConnection = nil,
		_PlayerRemovingConnection = nil,
		_RetryCount = 0,
		_LastSubmitTime = 0,
	}, MatchmakingClient) :: any

	-- Auto-setup player tracking
	Instance:_SetupPlayerTracking()

	return Instance
end

function MatchmakingClient.GetOptimalRegion(Self: MatchmakingClientInstance, TargetPlayer: Player): string
	if not Self.Configuration.AutoDetectRegion then
		return MatchmakingClient.Regions.NorthAmerica
	end

	local Success, CountryCode = pcall(function()
		return LocalizationService:GetCountryRegionForPlayerAsync(TargetPlayer)
	end)

	if Success and type(CountryCode) == "string" and RegionMapping[CountryCode] then
		Log(Self, "debug", `Detected region for {TargetPlayer.Name}: {RegionMapping[CountryCode]} (Country: {CountryCode})`)
		return RegionMapping[CountryCode]
	end

	Log(Self, "debug", `Could not detect region for {TargetPlayer.Name}, using NorthAmerica`)
	return MatchmakingClient.Regions.NorthAmerica
end

function MatchmakingClient.CreateParty(
	Self: MatchmakingClientInstance,
	LeaderId: PlayerId,
	LeaderName: string,
	LeaderRating: MatchmakingRating,
	PreferredRegion: Region?
): Party
	assert(ValidatePlayerId(LeaderId), "LeaderId must be a positive integer")
	assert(type(LeaderName) == "string" and LeaderName ~= "", "LeaderName must be a non-empty string")
	assert(ValidateRating(LeaderRating), "LeaderRating must be a valid number")

	local NewMember: PartyMember = {
		PlayerId = LeaderId,
		PlayerName = LeaderName,
		MatchmakingRating = LeaderRating,
	}

	local Region = PreferredRegion or MatchmakingClient.Regions.NorthAmerica

	return {
		Members = { NewMember },
		AverageMatchmakingRating = LeaderRating,
		PreferredRegion = Region,
		LeaderId = LeaderId,
	}
end

function MatchmakingClient.AddMemberToParty(
	Self: MatchmakingClientInstance,
	TargetParty: Party,
	NewPlayerId: PlayerId,
	NewPlayerName: string,
	NewPlayerRating: MatchmakingRating
): Party
	assert(ValidatePlayerId(NewPlayerId), "NewPlayerId must be a positive integer")
	assert(type(NewPlayerName) == "string" and NewPlayerName ~= "", "NewPlayerName must be a non-empty string")
	assert(ValidateRating(NewPlayerRating), "NewPlayerRating must be a valid number")

	local NewMember: PartyMember = {
		PlayerId = NewPlayerId,
		PlayerName = NewPlayerName,
		MatchmakingRating = NewPlayerRating,
	}

	-- Single-pass: clone and calculate rating
	local UpdatedMembers: { PartyMember } = {}
	local TotalRating: number = 0

	for _, Member in TargetParty.Members do
		table.insert(UpdatedMembers, Member)
		TotalRating += Member.MatchmakingRating
	end

	table.insert(UpdatedMembers, NewMember)
	TotalRating += NewPlayerRating

	return {
		Members = UpdatedMembers,
		AverageMatchmakingRating = TotalRating / #UpdatedMembers,
		PreferredRegion = TargetParty.PreferredRegion,
		LeaderId = TargetParty.LeaderId,
	}
end

function MatchmakingClient.RemoveMemberFromParty(
	Self: MatchmakingClientInstance,
	TargetParty: Party,
	TargetPlayerId: PlayerId
): Party?
	local UpdatedMembers: { PartyMember } = {}
	local TotalRating: number = 0

	for _, Member in TargetParty.Members do
		if Member.PlayerId ~= TargetPlayerId then
			table.insert(UpdatedMembers, Member)
			TotalRating += Member.MatchmakingRating
		end
	end

	if #UpdatedMembers == 0 then
		return nil
	end

	-- If leader was removed, make first remaining member the leader
	local NewLeaderId = TargetParty.LeaderId
	if TargetPlayerId == TargetParty.LeaderId then
		NewLeaderId = UpdatedMembers[1].PlayerId
	end

	return {
		Members = UpdatedMembers,
		AverageMatchmakingRating = TotalRating / #UpdatedMembers,
		PreferredRegion = TargetParty.PreferredRegion,
		LeaderId = NewLeaderId,
	}
end

function MatchmakingClient._MakeRequest(
	Self: MatchmakingClientInstance,
	Method: string,
	Endpoint: string,
	Body: { [string]: any }?
): (boolean, any, string?)
	assert(Endpoint and string.sub(Endpoint, 1, 1) == "/", "Endpoint must start with /")

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

		if Success then
			if Response and Response.Success then
				local DecodeSuccess: boolean, DecodedBody: any = pcall(function()
					return HttpService:JSONDecode(Response.Body)
				end)
				if DecodeSuccess then
					return true, DecodedBody, nil
				else
					return true, Response.Body, nil
				end
			else
				local ErrorMsg = if Response then `HTTP {Response.StatusCode}: {Response.Body}` else "Unknown error"
				if Attempt >= Attempts then
					return false, nil, ErrorMsg
				end
			end
		else
			local ErrorMsg = tostring(Response)
			if Attempt >= Attempts then
				return false, nil, ErrorMsg
			end
		end

		if Attempt < Attempts then
			task.wait(Self.Configuration.RetryDelaySeconds or DefaultRetryDelaySeconds)
		end
	end

	return false, nil, "Request failed after all retry attempts"
end

function MatchmakingClient.SubmitMatchmakingTicket(
	Self: MatchmakingClientInstance,
	TargetParty: Party,
	Options: SubmitOptions?
): MatchmakingResult
	-- Rate limiting
	local CurrentTime = tick()
	if CurrentTime - Self._LastSubmitTime < MinSubmitIntervalSeconds then
		return {
			Success = false,
			ErrorMessage = "Rate limited. Please wait before submitting another ticket.",
		}
	end
	Self._LastSubmitTime = CurrentTime

	-- Validate party
	if not TargetParty or not TargetParty.Members or #TargetParty.Members == 0 then
		return {
			Success = false,
			ErrorMessage = "Party must have at least one member",
		}
	end

	for Index, Member in TargetParty.Members do
		local Valid, Error = ValidatePartyMember(Member)
		if not Valid then
			return {
				Success = false,
				ErrorMessage = `Invalid member at index {Index}: {Error}`,
			}
		end
	end

	-- Validate options if provided
	if Options then
		if Options.GameMode then
			local Valid, Error = ValidateGameMode(Options.GameMode)
			if not Valid then
				return {
					Success = false,
					ErrorMessage = `Invalid GameMode: {Error}`,
				}
			end
		end

		if Options.Priority and (type(Options.Priority) ~= "number" or Options.Priority < 0 or Options.Priority > 10000) then
			return {
				Success = false,
				ErrorMessage = "Priority must be a number between 0 and 10000",
			}
		end
	end

	local RequestBody: { [string]: any } = {
		Members = TargetParty.Members,
		PreferredRegion = TargetParty.PreferredRegion,
	}

	if Options then
		if Options.GameMode then
			RequestBody.GameMode = Options.GameMode
		end
		if Options.Priority then
			RequestBody.Priority = Options.Priority
		end
		if Options.CustomData then
			RequestBody.CustomData = Options.CustomData
		end
	end

	local Success, Response, ErrorMsg = Self:_MakeRequest("POST", "/submit", RequestBody)

	if not Success then
		return {
			Success = false,
			ErrorMessage = `Failed to submit ticket: {ErrorMsg or "Unknown error"}`,
		}
	end

	-- Parse TicketId from response
	local TicketId: TicketId
	if type(Response) == "string" then
		TicketId = string.gsub(Response, '"', '')
	elseif type(Response) == "table" and Response.TicketId then
		TicketId = Response.TicketId
	else
		TicketId = tostring(Response)
	end

	-- Track ALL party members (not just single players)
	local MemberIds: { PlayerId } = {}
	for _, Member in TargetParty.Members do
		Self.ActiveTickets[Member.PlayerId] = TicketId
		table.insert(MemberIds, Member.PlayerId)
	end

	-- Track party info for leader-leave handling
	Self.ActiveParties[TicketId] = {
		TicketId = TicketId,
		LeaderId = TargetParty.LeaderId,
		MemberIds = MemberIds,
	}

	Self.TicketToPlayers[TicketId] = MemberIds

	Log(Self, "info", `Ticket submitted: {TicketId} ({#TargetParty.Members} players)`)

	return {
		Success = true,
		TicketId = TicketId,
	}
end

function MatchmakingClient.CancelMatchmakingTicket(
	Self: MatchmakingClientInstance,
	TargetTicketId: TicketId
): CancelResult
	if not TargetTicketId or TargetTicketId == "" then
		return {
			Success = false,
			ErrorMessage = "TicketId is required",
		}
	end

	local Success, _, ErrorMsg = Self:_MakeRequest("DELETE", `/ticket/{TargetTicketId}`, nil)

	-- Clean up tracking regardless of server response
	local PlayerIds = Self.TicketToPlayers[TargetTicketId]
	if PlayerIds then
		for _, PlayerId in PlayerIds do
			Self.ActiveTickets[PlayerId] = nil
		end
	end
	Self.TicketToPlayers[TargetTicketId] = nil
	Self.ActiveParties[TargetTicketId] = nil

	if Success then
		Log(Self, "info", `Ticket cancelled: {TargetTicketId}`)
		return { Success = true }
	else
		return {
			Success = false,
			ErrorMessage = ErrorMsg or "Failed to cancel ticket",
		}
	end
end

function MatchmakingClient.GetTicketStatus(
	Self: MatchmakingClientInstance,
	TargetTicketId: TicketId
): TicketStatusResult
	if not TargetTicketId or TargetTicketId == "" then
		return {
			Success = false,
			ErrorMessage = "TicketId is required",
		}
	end

	local Success, Response, ErrorMsg = Self:_MakeRequest("GET", `/ticket/{TargetTicketId}`, nil)

	if not Success then
		return {
			Success = false,
			ErrorMessage = ErrorMsg or "Failed to get ticket status",
		}
	end

	-- Validate response structure
	if type(Response) ~= "table" then
		return {
			Success = false,
			ErrorMessage = "Invalid response format",
		}
	end

	return {
		Success = true,
		Data = Response :: TicketStatus,
	}
end

function MatchmakingClient.CancelPlayerTicket(
	Self: MatchmakingClientInstance,
	TargetPlayerId: PlayerId
): CancelResult
	local TicketId: TicketId? = Self.ActiveTickets[TargetPlayerId]
	if not TicketId then
		return {
			Success = false,
			ErrorMessage = "No active ticket found for player",
		}
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
		Log(Self, "error", "Failed to decode match data")
		return
	end

	-- Validate match data structure
	if type(MatchData) ~= "table" then
		Log(Self, "error", "Invalid match data: not a table")
		return
	end

	if type(MatchData.MatchId) ~= "string" or MatchData.MatchId == "" then
		Log(Self, "error", "Invalid match data: missing or invalid MatchId")
		return
	end

	if type(MatchData.PlayerIds) ~= "table" or #MatchData.PlayerIds == 0 then
		Log(Self, "error", "Invalid match data: missing or empty PlayerIds")
		return
	end

	-- Validate each PlayerId
	for Index, PlayerId in MatchData.PlayerIds do
		if not ValidatePlayerId(PlayerId) then
			Log(Self, "error", `Invalid PlayerId at index {Index}`)
			return
		end
	end

	local ReceivedMatch: Match = {
		MatchId = MatchData.MatchId,
		PlayerIds = MatchData.PlayerIds,
	}

	-- Clean up tracking
	for _, PlayerId: PlayerId in ReceivedMatch.PlayerIds do
		local TicketId = Self.ActiveTickets[PlayerId]
		if TicketId then
			Self.ActiveParties[TicketId] = nil
			Self.TicketToPlayers[TicketId] = nil
		end
		Self.ActiveTickets[PlayerId] = nil
	end

	Log(Self, "info", `Match found: {ReceivedMatch.MatchId} ({#ReceivedMatch.PlayerIds} players)`)

	-- Call user callback safely
	if Self.OnMatchCreated then
		task.spawn(function()
			local CallbackSuccess, CallbackError = pcall(Self.OnMatchCreated, ReceivedMatch)
			if not CallbackSuccess then
				Log(Self, "error", `OnMatchCreated callback failed: {CallbackError}`)
			end
		end)
	end

	-- Auto teleport if enabled
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
		Log(Self, "warn", `No players found to teleport for match {TargetMatch.MatchId}`)
		return
	end

	local TeleportOptions: TeleportOptions = Instance.new("TeleportOptions")
	TeleportOptions.ShouldReserveServer = true

	local Success: boolean, ErrorResult: any = pcall(function()
		TeleportService:TeleportAsync(Self.Configuration.PlaceId, PlayersToTeleport, TeleportOptions)
	end)

	if Success then
		Log(Self, "info", `Teleported {#PlayersToTeleport} players for match {TargetMatch.MatchId}`)
	else
		Log(Self, "error", `Teleport failed for match {TargetMatch.MatchId}: {tostring(ErrorResult)}`)
	end
end

function MatchmakingClient._Subscribe(Self: MatchmakingClientInstance): boolean
	local Success, ConnectionOrError = pcall(function()
		return MessagingService:SubscribeAsync(Self.Configuration.Topic, function(Message)
			-- Wrap handler in pcall to prevent errors from breaking subscription
			local HandleSuccess, HandleError = pcall(function()
				Self:_HandleMatchFound(Message)
			end)
			if not HandleSuccess then
				Log(Self, "error", `Match handler error: {HandleError}`)
			end
		end)
	end)

	if Success and ConnectionOrError then
		Self._SubscriptionConnection = ConnectionOrError
		Self.IsListening = true
		Self._RetryCount = 0
		Log(Self, "info", `Listening on topic: {Self.Configuration.Topic}`)
		return true
	else
		Self.IsListening = false
		Log(Self, "error", `Failed to subscribe: {tostring(ConnectionOrError)}`)
		Self:_ScheduleResubscribe()
		return false
	end
end

function MatchmakingClient._ScheduleResubscribe(Self: MatchmakingClientInstance)
	if Self._RetryCount >= MaxRetryAttempts then
		Log(Self, "error", `Max subscription retry attempts ({MaxRetryAttempts}) reached`)
		return
	end

	Self._RetryCount += 1
	local Delay = RetryDelaySeconds * Self._RetryCount

	Log(Self, "warn", `Scheduling resubscribe attempt {Self._RetryCount} in {Delay} seconds`)

	task.delay(Delay, function()
		if not Self.IsListening and not Self._SubscriptionConnection then
			Self:_Subscribe()
		end
	end)
end

function MatchmakingClient.StartListening(Self: MatchmakingClientInstance): boolean
	if Self._SubscriptionConnection then
		Log(Self, "warn", "Already listening for matches")
		return false
	end

	return Self:_Subscribe()
end

function MatchmakingClient.StopListening(Self: MatchmakingClientInstance)
	if Self._SubscriptionConnection then
		Self._SubscriptionConnection:Disconnect()
		Self._SubscriptionConnection = nil
		Log(Self, "info", "Stopped listening for matches")
	end
	Self.IsListening = false
	Self._RetryCount = 0
end

function MatchmakingClient._SetupPlayerTracking(Self: MatchmakingClientInstance)
	-- Clean up existing connection if any
	if Self._PlayerRemovingConnection then
		Self._PlayerRemovingConnection:Disconnect()
	end

	Self._PlayerRemovingConnection = Players.PlayerRemoving:Connect(function(Player: Player)
		Self:OnPlayerLeft(Player.UserId)
	end)
end

function MatchmakingClient.OnPlayerLeft(Self: MatchmakingClientInstance, PlayerId: PlayerId): boolean
	local TicketId = Self.ActiveTickets[PlayerId]
	if not TicketId then
		return false
	end

	local PartyInfo = Self.ActiveParties[TicketId]
	if not PartyInfo then
		-- Single player ticket, just clean up
		Self.ActiveTickets[PlayerId] = nil
		return false
	end

	-- If leader left, cancel entire party's ticket
	if PartyInfo.LeaderId == PlayerId then
		Log(Self, "info", `Party leader {PlayerId} left, cancelling ticket {TicketId}`)

		-- Cancel on server (fire and forget)
		task.spawn(function()
			Self:CancelMatchmakingTicket(TicketId)
		end)

		-- Clean up all members locally
		for _, MemberId in PartyInfo.MemberIds do
			Self.ActiveTickets[MemberId] = nil
		end
		Self.ActiveParties[TicketId] = nil
		Self.TicketToPlayers[TicketId] = nil

		return true
	end

	-- Non-leader left - just remove from local tracking
	Self.ActiveTickets[PlayerId] = nil
	Log(Self, "debug", `Party member {PlayerId} left (ticket {TicketId} still active)`)
	return false
end

function MatchmakingClient.QueuePlayer(
	Self: MatchmakingClientInstance,
	TargetPlayer: Player,
	Rating: MatchmakingRating,
	PreferredRegion: Region?,
	Options: SubmitOptions?
): MatchmakingResult
	-- Auto-detect region if not provided
	local Region = PreferredRegion
	if not Region then
		Region = Self:GetOptimalRegion(TargetPlayer)
	end

	local Party: Party = Self:CreateParty(
		TargetPlayer.UserId,
		TargetPlayer.Name,
		Rating,
		Region
	)

	return Self:SubmitMatchmakingTicket(Party, Options)
end

function MatchmakingClient.DequeuePlayer(
	Self: MatchmakingClientInstance,
	TargetPlayer: Player
): CancelResult
	return Self:CancelPlayerTicket(TargetPlayer.UserId)
end

function MatchmakingClient.GetActiveTicketCount(Self: MatchmakingClientInstance): number
	local Count = 0
	for _ in Self.ActiveTickets do
		Count += 1
	end
	return Count
end

function MatchmakingClient.GetPlayerTicket(Self: MatchmakingClientInstance, PlayerId: PlayerId): TicketId?
	return Self.ActiveTickets[PlayerId]
end

function MatchmakingClient.Destroy(Self: MatchmakingClientInstance)
	Self:StopListening()

	if Self._PlayerRemovingConnection then
		Self._PlayerRemovingConnection:Disconnect()
		Self._PlayerRemovingConnection = nil
	end

	-- Clear all tracking
	table.clear(Self.ActiveTickets)
	table.clear(Self.ActiveParties)
	table.clear(Self.TicketToPlayers)

	Log(Self, "info", "MatchmakingClient destroyed")
end

return MatchmakingClient
