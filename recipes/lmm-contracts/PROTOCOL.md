# 固定 Protobuf 公共合同

历史声明基线：LMM API 固定提交 d468affecfd76a5123358732fb9a4e0cc1f63566，AGPL-3.0-or-later。以下为声明式服务/消息接口，不包含实现或执行钩子。保留原字段号与注释；声明存在不代表对应 RPC 已启用。

`payments.proto` 节同步本轮支付候选的完整声明，包含 `PreparePersonalTopup`、`GetEPayCheckout` 和显式提交方式；新增内容不重编号既有字段。最终固定源码及执行状态由 SOURCE.md 和主项目 STATUS.md 记录，不把候选声明归为旧提交的既有能力。

## lmm/core/v1/assistant.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";

// Core-owned assistant delegation. Each workflow requires its independently
// negotiated capability and a fully installed durable execution path.
// Every method reuses existing UDS host + original user metadata verification;
// there are no credentials, actor, payer, role, price or authority flags here.
service CoreAssistantModel {
  rpc BeginConversation(AssistantConversationKey) returns (AssistantConversationReceipt);
  rpc LookupConversation(AssistantConversationKey) returns (AssistantConversationReceipt);
  rpc IssueGrant(AssistantTurnKey) returns (AssistantGrantReceipt);
  rpc LookupGrant(AssistantTurnKey) returns (AssistantGrantReceipt);
  rpc ReserveText(AssistantTextReservation) returns (AssistantOperationReceipt);
  rpc GetOperation(AssistantOperationBinding) returns (AssistantOperationReceipt);
  rpc RevokeGrant(AssistantRevokeGrant) returns (AssistantRevokeReceipt);
  rpc GetNewUserGift(AssistantGiftRequest) returns (AssistantGiftStatus);
  rpc ClaimStoredNewUserGift(AssistantGiftRequest) returns (AssistantGiftClaim);
  rpc StartAgentTurn(AssistantAgentStart) returns (AssistantAgentTurn);
  rpc GetAgentTurn(AssistantAgentLookup) returns (AssistantAgentTurn);
  rpc ClaimAgentTool(AssistantAgentClaim) returns (AssistantAgentClaimReceipt);
  rpc CompleteAgentTool(AssistantAgentComplete) returns (AssistantAgentResult);
  rpc PrepareAssistantToolConfirmation(PrepareAssistantToolConfirmationRequest) returns (AssistantToolConfirmation);
  rpc GetAssistantToolConfirmationReceipt(GetAssistantToolConfirmationReceiptRequest) returns (AssistantToolConfirmation);
  rpc GetAssistantBusinessFacts(GetAssistantBusinessFactsRequest) returns (GetAssistantBusinessFactsResponse);
}
message GetAssistantBusinessFactsRequest {
  AssistantBusinessFactKind kind = 1;
}
enum AssistantBusinessFactKind {
  ASSISTANT_BUSINESS_FACT_KIND_UNSPECIFIED = 0;
  ASSISTANT_BUSINESS_FACT_KIND_AVAILABLE_MODELS = 1;
  ASSISTANT_BUSINESS_FACT_KIND_SUPPORT_ELIGIBILITY = 2;
}
message GetAssistantBusinessFactsResponse {
  bytes facts_json = 1; // Closed Core-owned projection, never financial authority.
}
message AssistantConversationKey { string creation_key = 1; }
message AssistantConversationReceipt { string creation_key = 1; string core_conversation_id = 2; }
message AssistantTurnKey { string core_conversation_id = 1; string turn_id = 2; }
message AssistantGrantReceipt {
  string delegation_id = 1;
  string core_conversation_id = 2;
  string turn_id = 3;
  int64 expires_at_unix = 4;
  uint32 max_model_calls = 5;
  bool revoked = 6;
}
message AssistantOperationBinding {
  string delegation_id = 1;
  string core_conversation_id = 2;
  string turn_id = 3;
  uint32 round = 4;
  string operation_key = 5; // exact turn_id + ':' + round; 0 <= round < 4
  bytes text_sha256 = 6; // SHA256 of exact UTF8 text bytes, NOT legacy request_json.
}
message AssistantTextReservation {
  AssistantOperationBinding binding = 1;
  string text = 2; // bounded <=8192 UTF8 bytes; no tools/results supported yet.
}
enum AssistantOperationState {
  ASSISTANT_OPERATION_STATE_UNSPECIFIED = 0;
  ASSISTANT_OPERATION_STATE_RESERVED = 1;
  ASSISTANT_OPERATION_STATE_STREAMING = 2;
  ASSISTANT_OPERATION_STATE_SETTLED = 3;
  ASSISTANT_OPERATION_STATE_RELEASED = 4;
  ASSISTANT_OPERATION_STATE_UNKNOWN = 5;
}
message AssistantOperationReceipt {
  AssistantOperationBinding binding = 1;
  string request_id = 2;
  AssistantOperationState state = 3;
  int64 reserved_credits = 4;
  optional int64 settled_credits = 5; // present only for authoritative settled.
  // Present only for SETTLED, persisted in the same transaction as settlement.
  // Replays and UNKNOWN inspect the original operation; they never dispatch again.
  optional string answer_text = 6;
  AssistantGiftReceipt gift_receipt = 7; // only SETTLED, durable authoritative Core gift outcome.
}
message AssistantRevokeGrant { string delegation_id = 1; }
message AssistantRevokeReceipt { string delegation_id = 1; bool revoked = 2; }
message AssistantGiftRequest {}
enum AssistantGiftState {
  ASSISTANT_GIFT_STATE_UNSPECIFIED = 0;
  ASSISTANT_GIFT_STATE_OFFERED = 1;
  ASSISTANT_GIFT_STATE_CLAIMED = 2;
}
message AssistantGiftReceipt {
  string gift_id = 1; // 64 lowercase hexadecimal decision id, not a bearer token.
  int64 owner_user_id = 2; // Core authenticated original Session user.
  AssistantGiftState state = 3;
  int64 credit_amount = 4; // integer ledger credits, 500000 per USD anchor.
  int64 created_at_unix = 5;
  int64 claimed_at_unix = 6;
  string ledger_transaction_id = 7; // positive decimal journal id only for CLAIMED.
  int64 max_credit_amount = 8; // current exclusive policy cap; may differ from old grant.
  bool claim_available = 9; // current Core eligibility; never browser authority.
  string claim_blocked_code = 10; // safe closed policy reason, no identity/network data.
}
message AssistantGiftStatus { AssistantGiftReceipt gift = 1; }
message AssistantGiftClaim { AssistantGiftReceipt gift = 1; bool already_claimed = 2; }

message AssistantAgentLookup {
  string core_conversation_id = 1;
  string client_turn_id = 2;
}
message AssistantAgentStart {
  string core_conversation_id = 1;
  string client_turn_id = 2;
  string text = 3;
}
enum AssistantAgentPhase {
  ASSISTANT_AGENT_PHASE_UNSPECIFIED = 0;
  ASSISTANT_AGENT_PHASE_NEW = 1;
  ASSISTANT_AGENT_PHASE_MODEL_RUNNING = 2;
  ASSISTANT_AGENT_PHASE_TOOLS_PENDING = 3;
  ASSISTANT_AGENT_PHASE_COMPLETED = 4;
  ASSISTANT_AGENT_PHASE_UNKNOWN = 5;
  ASSISTANT_AGENT_PHASE_REJECTED = 6;
  ASSISTANT_AGENT_PHASE_EXPIRED = 7;
}
enum AssistantAgentDomain {
  ASSISTANT_AGENT_DOMAIN_UNSPECIFIED = 0;
  ASSISTANT_AGENT_DOMAIN_REGISTERED_CORE = 1;
  ASSISTANT_AGENT_DOMAIN_EXTENSION = 2;
}
message AssistantAgentScope {
  string tool_name = 1;
  string tool_version = 2;
  bytes schema_sha256 = 3;
  string price_version = 4;
  bytes price_sha256 = 5;
  string resource_kind = 6;
  string resource_id = 7;
  string resource_version = 8;
  string grant_id = 9;
  AssistantAgentDomain domain = 10;
  bytes resource_schema_sha256 = 11;
  bytes resource_price_sha256 = 12;
}
message AssistantAgentToolCall {
  string tool_operation_id = 1;
  string provider_call_id = 2;
  string model_operation_id = 3;
  uint32 round = 4;
  uint32 ordinal = 5;
  AssistantAgentScope scope = 6;
  bytes arguments_json = 7;
  bytes arguments_sha256 = 8;
}
message AssistantAgentTurn {
  string core_conversation_id = 1;
  string client_turn_id = 2;
  int64 owner_user_id = 3;
  int64 owner_account_id = 4;
  bytes original_text_sha256 = 5;
  AssistantAgentPhase phase = 6;
  uint64 revision = 7;
  uint32 round = 8;
  uint32 max_rounds = 9;
  int64 model_budget_credits = 10;
  int64 model_held_credits = 11;
  int64 model_settled_credits = 12;
  repeated AssistantAgentToolCall pending_calls = 13;
  optional string final_text = 14;
}
message AssistantAgentClaim {
  string tool_operation_id = 1;
  string claim_key = 2;
}
enum AssistantAgentClaimDisposition {
  ASSISTANT_AGENT_CLAIM_UNSPECIFIED = 0;
  ASSISTANT_AGENT_CLAIM_EXECUTE_ONCE = 1;
  ASSISTANT_AGENT_CLAIM_INSPECT_ONLY = 2;
}
message AssistantAgentClaimReceipt {
  AssistantAgentToolCall call = 1;
  string claim_key = 2;
  AssistantAgentClaimDisposition disposition = 3;
  optional AssistantAgentResult accepted_result = 4;
}
enum AssistantAgentResultState {
  ASSISTANT_AGENT_RESULT_UNSPECIFIED = 0;
  ASSISTANT_AGENT_RESULT_COMPLETED = 1;
  ASSISTANT_AGENT_RESULT_UNKNOWN = 2;
}
message AssistantAgentComplete {
  string tool_operation_id = 1;
  string claim_key = 2;
  AssistantAgentResultState state = 3;
  bytes result_json = 4;
  bytes result_sha256 = 5;
  string domain_receipt_id = 6;
}
message AssistantAgentResult {
  string tool_operation_id = 1;
  string claim_key = 2;
  AssistantAgentResultState state = 3;
  bytes result_sha256 = 4;
  string domain_receipt_id = 5;
}

message PrepareAssistantToolConfirmationRequest {
 string tool_operation_id = 1;
 bytes normalized_draft_json = 2; // bounded 16 KiB object; per-tool Core validator
}
message GetAssistantToolConfirmationReceiptRequest {
 string tool_operation_id = 1;
 bytes normalized_draft_sha256 = 2; // exactly 32 bytes, returned by Core Prepare
}
message AssistantToolConfirmation {
 string challenge_id = 1;
 string tool_operation_id = 2;
 string tool_name = 3;
 string tool_version = 4;
 bytes source_arguments_sha256 = 5;
 bytes normalized_draft_sha256 = 6;
 bytes preview_json = 7;
 string resource_kind = 8;
 string resource_id = 9;
 string resource_version = 10;
 int64 expires_at_unix = 11;
 string state = 12; // pending/approved/rejected; GetReceipt returns approved only
 string browser_path = 13;
 bool executed = 14; // always false; domain receipt proves actual execution
}
```

## lmm/core/v1/commands.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";
import "lmm/core/v1/control.proto";

// Integration contracts, NOT available business implementations. Servers must
// advertise a feature only after its business module and authorization exist.
// Every mutation requires independent service AND current user authorization.
// Recheck user/resource/funding authority in the business transaction, including
// replay. A transport credential or an idempotency key never grants authority.
message MutationContext {
  string idempotency_key = 1; // 16..128 visible ASCII bytes; stable across retries.
  optional int64 expected_version = 2;
}
message MutationReceipt {
  int64 operation_id = 1;
  int64 resource_id = 2;
  int64 resource_version = 3;
  repeated int64 event_ids = 4;
}
message LedgerAmount {
  int64 units = 1; // Positive integer units, defined by the core's asset registry.
  string asset = 2; // Allowlisted asset; no client-selected precision or float.
}
service CoreIdentityCommands {
  rpc RevokeCredential(RevokeCredentialRequest) returns (MutationReceipt);
}
message RevokeCredentialRequest {
  MutationContext mutation = 1;
  int64 credential_id = 2;
}
service CoreLedgerCommands {
  rpc CaptureVerifiedPayment(CaptureVerifiedPaymentRequest) returns (MutationReceipt);
  rpc Transfer(TransferFundsRequest) returns (MutationReceipt);
  rpc Refund(RefundFundsRequest) returns (MutationReceipt);
}
message CaptureVerifiedPaymentRequest {
  MutationContext mutation = 1;
  // A core-verified payment record fixes payer, amount and external event ID.
  // This request cannot supply a replacement account or credit amount.
  int64 verified_payment_id = 2;
}
message TransferFundsRequest {
  MutationContext mutation = 1;
  Account source = 2;
  Account destination = 3;
  LedgerAmount amount = 4;
}
message RefundFundsRequest {
  MutationContext mutation = 1;
  int64 original_operation_id = 2;
  LedgerAmount amount = 3; // Core caps cumulative refunds at the original debit.
}
service CoreBillingCommands {
  rpc Reserve(ReserveUsageRequest) returns (ReservationReceipt);
  rpc Settle(SettleUsageRequest) returns (MutationReceipt);
  rpc Release(ReleaseUsageRequest) returns (MutationReceipt);
}
message ReserveUsageRequest {
  MutationContext mutation = 1;
  // Quote and model request are already core-owned. Core selects funding and
  // applies all user/team/key budgets; callers cannot choose a balance to debit.
  int64 quote_id = 2;
  string model_request_id = 3;
}
message ReservationReceipt {
  MutationReceipt mutation = 1;
  int64 reservation_id = 2;
  Account charged_account = 3;
}
message UsageCounters {
  int64 input_tokens = 1;
  int64 output_tokens = 2;
  int64 cache_read_tokens = 3;
  int64 cache_write_tokens = 4;
}
message SettleUsageRequest {
  MutationContext mutation = 1;
  int64 reservation_id = 2;
  // Only the trusted core relay's validated usage evidence may settle. A Go
  // module/user cannot submit arbitrary usage to reduce the charge.
  int64 verified_usage_id = 3;
}
message ReleaseUsageRequest {
  MutationContext mutation = 1;
  int64 reservation_id = 2;
}
service CoreRoutingCommands {
  rpc ActivateConfiguration(ActivateConfigurationRequest) returns (MutationReceipt);
}
message ActivateConfigurationRequest {
  MutationContext mutation = 1;
  // A staged, validated immutable configuration owned by core. CAS activation
  // must preserve the previous active version on failure. No arbitrary settings.
  int64 validated_configuration_id = 2;
}
```

## lmm/core/v1/commerce.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";

// Narrow store port. The independently configured commerce service credential
// AND x-lmm-user-credential are required on Execute/Query, including replay.
// This service cannot mint credit, select arbitrary ledger entries or borrow.
service CoreCommerce {
  rpc ExecuteCommerce(ExecuteCommerceRequest) returns (CommerceReceipt);
  rpc QueryCommerce(QueryCommerceRequest) returns (QueryCommerceResponse);
  // Uses only the original buyer-authorized, fixed-order capture grant. No new
  // spend, reserve, payer, amount or recipient can be selected. This separate
  // service permission is disabled by default; no user bearer is retained.
  rpc CompleteDelivery(CompleteDeliveryRequest) returns (CommerceReceipt);
  // Separate service-only READ permission for lost-ACK recovery. Exact original
  // command bytes must match the stored operation, under the same trusted
  // merchant principal. Missing receipt is UNKNOWN. Never creates anything.
  rpc ReconcileCommerce(ExecuteCommerceRequest) returns (QueryCommerceResponse);
}
message ExecuteCommerceRequest {
  uint32 protocol_major = 1;
  // Exact json.Marshal([]any{store.MoneyCommand}) bytes; one command only.
  // Core validates the complete typed command and computes SHA256 of these
  // bytes, matching store.ID(command). Caller-supplied hashes are not trusted.
  bytes command_json = 2;
}
message QueryCommerceRequest {
  uint32 protocol_major = 1;
  string shop_id = 2;
  string order_id = 3;
  string operation_key = 4;
}
message CommerceReceipt {
  string key = 1;
  string command_hash = 2;
  string reference = 3;
  string outcome = 4; // Exactly succeeded or permanently rejected.
  string rejection_code = 5;
}
message QueryCommerceResponse {
  // Missing receipt means UNKNOWN, never rejection or permission to compensate.
  optional CommerceReceipt receipt = 1;
}
message CompleteDeliveryRequest {
  uint32 protocol_major = 1;
  // The original order's frozen MoneyCommand with kind="settle" and exact
  // original payment_ref. Command key/hash match ExecuteCommerce semantics.
  bytes command_json = 2;
  string payment_command_hash = 3;
  // Stable bounded ID of the Go-owned, already committed delivery fact. This
  // is audit evidence from the merchant service, not independent Core inspection.
  string delivery_reference = 4;
}
```

## lmm/core/v1/control.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";

// Internal, read-only control plane. Not a model relay or a spending grant.
// Service and user credentials travel in separate metadata, never in messages.
// Keep existing field numbers and meanings. Reserve removed numbers.
service CoreControl {
  rpc Capabilities(CapabilitiesRequest) returns (CapabilitiesResponse);
  rpc Authorize(AuthorizeRequest) returns (AuthorizeResponse);
  rpc ListTeams(ListTeamsRequest) returns (ListTeamsResponse);
  // Purpose-specific remote authority; not general identity or spending.
  rpc AuthorizeRemote(AuthorizeRemoteRequest) returns (AuthorizeRemoteResponse);
}
message CapabilitiesRequest {}
message CapabilitiesResponse {
  uint32 protocol_major = 1;
  bool identity_available = 2;
  repeated string features = 3;
}
message AuthorizeRequest {}
enum AccountKind {
  ACCOUNT_KIND_UNSPECIFIED = 0;
  ACCOUNT_KIND_PERSONAL = 1;
  ACCOUNT_KIND_TEAM = 2;
}
message Account {
  AccountKind kind = 1;
  int64 id = 2;
}
enum CredentialKind {
  CREDENTIAL_KIND_UNSPECIFIED = 0;
  CREDENTIAL_KIND_SESSION = 1;
  CREDENTIAL_KIND_API_KEY = 2;
}
message AuthorizeResponse {
  int64 credential_id = 1;
  CredentialKind credential_kind = 2;
  int64 user_id = 3;
  int32 platform_level = 4;
  Account owner = 5;
  // Ordered candidates only. Billing must recheck authority and reserve funds.
  repeated Account funding_accounts = 6;
}
enum TeamRole {
  TEAM_ROLE_UNSPECIFIED = 0;
  TEAM_ROLE_OWNER = 1;
  TEAM_ROLE_ADMIN = 2;
  TEAM_ROLE_MEMBER = 3;
}
// Authentication is forwarded in server-owned metadata, never caller scopes/owner.
message AuthorizeRemoteRequest {}
message NativePersonalRemoteSource {
  int64 credential_id = 1;
  // Only interactive PERSONAL SESSION is currently authorized, not API_KEY.
  CredentialKind native_kind = 2;
}
message OAuthGrantRemoteSource {
  string grant_id = 1;
  string access_id = 2;
}
message AuthorizeRemoteResponse {
  oneof source {
    NativePersonalRemoteSource native_personal = 1;
    OAuthGrantRemoteSource oauth_grant = 2;
  }
  int64 user_id = 3;
  // Stable PERSONAL storage account, not a public user/team ID.
  int64 owner_account_id = 4;
  int64 auth_version = 5;
  int64 expires_at = 6;
  // Revalidate at <=15s; expiry does not permit caching through revocation.
  bool remote_permission = 7;
}
message ListTeamsRequest {
  // Exclusive cursor, nonnegative. Results are ordered by team ID, at most 100.
  int64 after_id = 1;
}
message Team {
  int64 id = 1;
  TeamRole role = 2;
  int64 membership_version = 3;
  bool can_spend = 4;
}
message ListTeamsResponse {
  repeated Team teams = 1;
  // Zero means the end; an empty final page is allowed after a full page.
  int64 next_after_id = 2;
}
```

## lmm/core/v1/events.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";

// Durable, service-scoped subscriptions. No user or service secrets in messages.
// Subscriptions are provisioned by core configuration, NOT by this service.
service CoreEvents {
  rpc Negotiate(NegotiateEventsRequest) returns (NegotiateEventsResponse);
  rpc Pull(PullEventsRequest) returns (PullEventsResponse);
  rpc Acknowledge(AcknowledgeEventRequest) returns (AcknowledgeEventResponse);
  rpc Retry(RetryEventRequest) returns (RetryEventResponse);
}
message EventClientVersion {
  uint32 protocol_major = 1;
  uint32 protocol_minor = 2;
  repeated string required_features = 3;
}
message NegotiateEventsRequest {
  string consumer_id = 1;
  EventClientVersion version = 2;
}
message NegotiateEventsResponse {
  uint32 protocol_major = 1;
  uint32 protocol_minor = 2;
  repeated string features = 3;
  uint32 max_batch = 4;
  uint32 max_payload_bytes = 5;
  uint32 lease_seconds = 6;
  repeated string event_types = 7;
}
// All IDs are positive signed 64-bit integers. Never pass them through a float.
message EventEnvelope {
  int64 event_id = 1;
  string event_key = 2;
  string event_type = 3;
  uint32 schema_version = 4;
  int64 resource_id = 5;
  int64 resource_version = 6;
  // Original serialized domain message. Store/forward without re-encoding.
  bytes payload = 7;
  // Stable digest; algorithm is specified in contracts/proto/README.md.
  bytes content_sha256 = 8;
}
message EventDelivery {
  EventEnvelope event = 1;
  bytes lease_token = 2;
  uint32 attempt = 3;
  int64 lease_until_unix_ms = 4;
}
message PullEventsRequest {
  string consumer_id = 1;
  EventClientVersion version = 2;
  uint32 max_events = 3;
}
message PullEventsResponse { repeated EventDelivery deliveries = 1; }
message AcknowledgeEventRequest {
  string consumer_id = 1;
  int64 event_id = 2;
  bytes lease_token = 3;
}
message AcknowledgeEventResponse { bool already_acknowledged = 1; }
enum EventRetryReason {
  EVENT_RETRY_REASON_UNSPECIFIED = 0;
  EVENT_RETRY_REASON_HANDLER_FAILURE = 1;
  EVENT_RETRY_REASON_UNSUPPORTED_EVENT = 2;
  EVENT_RETRY_REASON_DEPENDENCY_UNAVAILABLE = 3;
}
message RetryEventRequest {
  string consumer_id = 1;
  int64 event_id = 2;
  bytes lease_token = 3;
  EventRetryReason reason = 4;
}
message RetryEventResponse { bool already_acknowledged = 1; }
```

## lmm/core/v1/ledger.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";
import "lmm/core/v1/commands.proto";

// Task 02 handoff. Contract only: do not register before user/team/key budgets,
// service permissions and same-transaction outbox integration exist.
service CoreLedger {
  rpc PostLedger(PostLedgerRequest) returns (PostLedgerResponse);
  rpc GetLedgerBalance(GetLedgerBalanceRequest) returns (GetLedgerBalanceResponse);
}
message PostLedgerRequest {
  uint32 protocol_major = 1; // Must be 1; unknown action fails closed.
  MutationContext mutation = 2;
  string unit = 3; // Exactly credit_500k_usd: 500,000 integer units = USD 1.
  string business_reference = 4; // Stable, bounded reference; not authorization.
  string audit_reason = 5; // Bounded text, no secrets.
  // Scope and actor come from independent service/user authentication, never a
  // supplied actor ID. Each action has a separate ledger.<action> permission.
  oneof action {
    LedgerCredit credit = 10;
    LedgerTransfer transfer = 11;
    LedgerCharge charge = 12;
    LedgerReserve reserve = 13;
    LedgerCapture capture = 14;
    LedgerRelease release = 15;
    LedgerRefund refund = 16;
    LedgerReverse reverse = 17;
  }
}
message LedgerCredit {
  // Core-verified record fixes account, amount, conversion and globally unique
  // provider/merchant/environment/transaction. No caller-chosen credit amount.
  int64 verified_payment_id = 1;
}
message LedgerTransfer {
  int64 source_account_id = 1; // core_identity.accounts.id, NOT user/team ID.
  int64 destination_account_id = 2;
  int64 amount = 3; // Positive units. Check both ownership and treasury scope.
}
message LedgerCharge {
  int64 billing_authorization_id = 1; // Core-owned durable single-use grant.
}
message LedgerReserve {
  int64 billing_authorization_id = 1; // Fixes payer, amount and all budget holds.
}
message LedgerCapture {
  int64 reservation_journal_id = 1;
  int64 verified_usage_id = 2; // Core computes amount; zero final charge is valid.
}
message LedgerRelease {
  int64 reservation_journal_id = 1;
  int64 terminal_request_evidence_id = 2; // Timeout alone does not allow release.
}
message LedgerRefund {
  int64 original_journal_id = 1;
  int64 amount = 2; // Positive; cumulative cap; destination fixed by original.
}
message LedgerReverse {
  int64 original_journal_id = 1; // Narrow reconciliation/admin permission only.
}
message PostLedgerResponse {
  string idempotency_key = 1;
  bytes request_sha256 = 2; // Core-computed canonical request, including actor.
  string receipt_id = 3; // Stable across retries, including permanent rejection.
  oneof outcome {
    LedgerPosted posted = 10;
    LedgerRejected rejected = 11;
  }
  // Idempotency conflict is ALREADY_EXISTS, not a business rejection.
  // UNAVAILABLE/DEADLINE_EXCEEDED may mean UNKNOWN commit: repeat the same key.
  // Do not infer failure from an absent response or create a compensating debit.
}
message LedgerPosted {
  int64 journal_id = 1;
  string unit = 2;
  repeated LedgerEntrySnapshot entries = 3;
}
message LedgerEntrySnapshot {
  int64 ledger_account_id = 1;
  int64 owner_account_id = 2;
  int64 delta = 3;
  int64 resulting_balance = 4;
  int64 revision = 5;
}
message LedgerRejected {
  string code = 1; // Stable domain code; permanent result for this exact key.
}
message GetLedgerBalanceRequest {
  int64 account_id = 1; // Require ledger.read and current account authorization.
}
message GetLedgerBalanceResponse {
  int64 account_id = 1;
  string unit = 2;
  int64 available = 3;
  int64 held = 4;
  int64 available_revision = 5;
  int64 held_revision = 6;
}
message LedgerPostedEvent {
  string receipt_id = 1;
  string idempotency_key = 2;
  LedgerPosted posted = 3;
}
```

## lmm/core/v1/payments.proto

```protobuf
syntax = "proto3";
package lmm.core.v1;
option go_package = "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb;corepb";
import "lmm/core/v1/commands.proto";

// Task 10 handoff, NOT registered. User actions need current user authorization
// plus service permissions. Provider-driven transitions require independently
// verified provider evidence and a core-issued intent/authority, not merely a
// service token. Go cannot assert that a provider event has been verified.
service CorePayments {
  rpc AcceptProviderProof(AcceptProviderProofRequest) returns (ProviderProofReceipt);
  rpc GetProviderProof(GetProviderProofRequest) returns (ProviderProofReceipt);
  rpc PrepareTopup(PrepareTopupRequest) returns (PaymentIntent);
  // Resolve the personal payer from the current Core session, never a web ID.
  rpc PreparePersonalTopup(PreparePersonalTopupRequest) returns (PaymentIntent);
  // EPay shares one MD5 key between checkout and callback verification. Keep
  // that key in Core; only sign the fixed fields of an existing Core intent.
  rpc GetEPayCheckout(GetEPayCheckoutRequest) returns (EPayCheckout);
  rpc CreditTopup(CreditTopupRequest) returns (PaymentReceipt);
  rpc HoldRefund(HoldRefundRequest) returns (PaymentReceipt);
  rpc BeginRefundDispatch(ResolveRefundRequest) returns (PaymentReceipt);
  rpc CommitRefund(ResolveRefundRequest) returns (PaymentReceipt);
  rpc ReleaseRefund(ResolveRefundRequest) returns (PaymentReceipt);
  rpc ApplyExternalRefund(ExternalRefundRequest) returns (PaymentReceipt);
  rpc GetPaymentReceipt(GetPaymentReceiptRequest) returns (PaymentReceipt);
}
message ExternalPaymentReference {
  string provider = 1;
  string merchant = 2;
  string environment = 3;
  string transaction = 4;
}
message ProviderEvidence {
  ExternalPaymentReference payment = 1;
  // Bounded original bytes and signature from a configured adapter. Core must
  // independently verify signature, merchant, amount, currency, event identity
  // and replay state. No endpoint URL, secret or caller-set 'verified' flag.
  bytes signed_payload = 2;
  bytes signature = 3;
  string provider_event_id = 4;
  // Only Rust can issue this durable ingress receipt. Later settlement uses
  // this reference and the exact original payload, never a caller timestamp.
  string accepted_proof_id = 5;
}
message AcceptProviderProofRequest {
  int64 intent_id = 1;
  ProviderEvidence evidence = 2;
}
message GetProviderProofRequest {
  int64 intent_id = 1;
  string provider_event_id = 2;
  bytes payload_sha256 = 3;
}
// Receiving proof is distinct from applying money. There is deliberately no
// journal or payment-success state here. Raw bytes/signatures stay private.
message ProviderProofReceipt {
  string proof_id = 1;
  int64 intent_id = 2;
  string provider_event_id = 3;
  bytes payload_sha256 = 4;
  ExternalPaymentReference payment = 5;
  int64 accepted_unix_seconds = 6;
}
message PrepareTopupRequest {
  MutationContext mutation = 1;
  int64 account_id = 2; // core_identity.accounts.id; explicit personal/team account.
  int64 payment_channel_id = 3; // Core-owned allowlisted merchant configuration.
  string currency = 4; // ISO code, e.g. USD. Core validates channel and currency.
  int64 amount_minor = 5; // Requested positive external smallest units, not credit.
}
message PreparePersonalTopupRequest {
  MutationContext mutation = 1;
  int64 payment_channel_id = 2;
  string currency = 3;
  int64 amount_minor = 4;
}
message PaymentIntent {
  int64 intent_id = 1;
  int64 account_id = 2;
  int64 created_by_user_id = 3; // Output only, from authenticated identity.
  string provider = 4;
  string merchant = 5;
  string environment = 6;
  string currency = 7;
  int64 amount_minor = 8;
  int64 credit_units = 9; // Core-frozen conversion/policy, never Go-calculated.
  string credit_unit = 10; // credit_500k_usd.
  string idempotency_key = 11;
  bytes request_sha256 = 12;
  string receipt_id = 13;
}
message GetEPayCheckoutRequest {
  int64 intent_id = 1;
}
message EPayCheckout {
  int64 intent_id = 1;
  string order_id = 2;
  string checkout_url = 3;
  // GET opens checkout_url; POST submits its query fields to its queryless URL.
  string method = 4;
}
message CreditTopupRequest {
  MutationContext mutation = 1;
  int64 intent_id = 2;
  ProviderEvidence evidence = 3;
}
message HoldRefundRequest {
  MutationContext mutation = 1;
  int64 intent_id = 2; // Must locate original credit even if Go lost its response.
  int64 refund_amount_minor = 3;
}
message ResolveRefundRequest {
  MutationContext mutation = 1;
  int64 intent_id = 2;
  string hold_receipt_id = 3;
  // Commit requires verified provider success; release requires verified terminal
  // failure. Missing/unknown evidence MUST NOT release a hold on timeout.
  ProviderEvidence evidence = 4;
}
message ExternalRefundRequest {
  MutationContext mutation = 1;
  int64 intent_id = 2;
  ProviderEvidence evidence = 3;
  // Must handle a pre-credit refund, a lost credit response and an existing
  // refund hold atomically. Persist a fence against later stale credit replay.
}
message GetPaymentReceiptRequest {
  int64 intent_id = 1;
  string idempotency_key = 2; // Resolved inside authenticated scope, not global.
  // Require topup.read or explicitly scoped reconcile authority.
}
enum PaymentReceiptState {
  PAYMENT_RECEIPT_STATE_UNSPECIFIED = 0;
  PAYMENT_RECEIPT_STATE_CREDIT_APPLIED = 1;
  PAYMENT_RECEIPT_STATE_REFUND_HELD = 2;
  PAYMENT_RECEIPT_STATE_REFUND_COMMITTED = 3;
  PAYMENT_RECEIPT_STATE_REFUND_RELEASED = 4;
  PAYMENT_RECEIPT_STATE_EXTERNAL_REFUND_APPLIED = 5;
  PAYMENT_RECEIPT_STATE_REJECTED = 6;
  // Only the core-issued intent was durably saved. No credit/hold/transfer has
  // occurred; clients MUST NOT interpret this as accepted or queued money.
  PAYMENT_RECEIPT_STATE_PREPARED = 7;
}
message PaymentReceipt {
  string receipt_id = 1;
  int64 intent_id = 2;
  string idempotency_key = 3;
  bytes request_sha256 = 4;
  PaymentReceiptState state = 5;
  int64 journal_id = 6; // When posted; cannot infer a credit from this field alone.
  string rejection_code = 7;
  // No accepted/queued success state. Return only durably committed states.
  // Transport/database timeout remains an unknown result; query/repeat same key.
}
message PaymentStateChangedEvent {
  PaymentReceipt receipt = 1;
  int64 account_id = 2;
}
```

## 付款候选黄金向量

来自 contracts/core/v1/funding-cases.json，供 provider 与 consumer 一致性检查。

```json
[
  {
    "name": "personal-default",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": null,
    "grants": [],
    "expected": [
      {
        "kind": "personal",
        "id": 7
      }
    ]
  },
  {
    "name": "joining-team-does-not-expand-default",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": null,
    "grants": [
      42
    ],
    "expected": [
      {
        "kind": "personal",
        "id": 7
      }
    ]
  },
  {
    "name": "explicit-personal-only",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "personal",
        "id": 7
      }
    ],
    "grants": [],
    "expected": [
      {
        "kind": "personal",
        "id": 7
      }
    ]
  },
  {
    "name": "empty-is-not-default",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [],
    "grants": [],
    "error": "invalid_order"
  },
  {
    "name": "personal-team-personal-team-order",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "team",
        "id": 42
      },
      {
        "kind": "personal",
        "id": 7
      },
      {
        "kind": "team",
        "id": 91
      }
    ],
    "grants": [
      42,
      91
    ],
    "expected": [
      {
        "kind": "team",
        "id": 42
      },
      {
        "kind": "personal",
        "id": 7
      },
      {
        "kind": "team",
        "id": 91
      }
    ]
  },
  {
    "name": "personal-may-omit-own-wallet",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "team",
        "id": 42
      }
    ],
    "grants": [
      42
    ],
    "expected": [
      {
        "kind": "team",
        "id": 42
      }
    ]
  },
  {
    "name": "personal-cannot-spend-another-person",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "personal",
        "id": 8
      }
    ],
    "grants": [],
    "error": "not_authorized"
  },
  {
    "name": "validate-entire-order-before-charge",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "personal",
        "id": 7
      },
      {
        "kind": "team",
        "id": 42
      }
    ],
    "grants": [],
    "error": "not_authorized"
  },
  {
    "name": "revoked-team-not-silently-removed",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "team",
        "id": 42
      },
      {
        "kind": "personal",
        "id": 7
      }
    ],
    "grants": [],
    "error": "not_authorized"
  },
  {
    "name": "duplicate-owner",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "personal",
        "id": 7
      },
      {
        "kind": "personal",
        "id": 7
      }
    ],
    "grants": [],
    "error": "invalid_order"
  },
  {
    "name": "duplicate-team",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "team",
        "id": 42
      },
      {
        "kind": "team",
        "id": 42
      }
    ],
    "grants": [
      42
    ],
    "error": "invalid_order"
  },
  {
    "name": "same-id-distinct-kinds",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "team",
        "id": 7
      },
      {
        "kind": "personal",
        "id": 7
      }
    ],
    "grants": [
      7
    ],
    "expected": [
      {
        "kind": "team",
        "id": 7
      },
      {
        "kind": "personal",
        "id": 7
      }
    ]
  },
  {
    "name": "team-default-needs-grant",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": null,
    "grants": [],
    "error": "not_authorized"
  },
  {
    "name": "team-default-authorized",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": null,
    "grants": [
      42
    ],
    "expected": [
      {
        "kind": "team",
        "id": 42
      }
    ]
  },
  {
    "name": "team-explicit-owner",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": [
      {
        "kind": "team",
        "id": 42
      }
    ],
    "grants": [
      42
    ],
    "expected": [
      {
        "kind": "team",
        "id": 42
      }
    ]
  },
  {
    "name": "team-no-founders-wallet",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": [
      {
        "kind": "personal",
        "id": 7
      }
    ],
    "grants": [
      42
    ],
    "error": "invalid_order"
  },
  {
    "name": "team-no-extra-team",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": [
      {
        "kind": "team",
        "id": 42
      },
      {
        "kind": "team",
        "id": 91
      }
    ],
    "grants": [
      42,
      91
    ],
    "error": "invalid_order"
  },
  {
    "name": "team-no-other-team-even-with-grant",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": [
      {
        "kind": "team",
        "id": 91
      }
    ],
    "grants": [
      42,
      91
    ],
    "error": "invalid_order"
  },
  {
    "name": "team-no-empty-order",
    "owner": {
      "kind": "team",
      "id": 42
    },
    "configured": [],
    "grants": [
      42
    ],
    "error": "invalid_order"
  },
  {
    "name": "zero-owner",
    "owner": {
      "kind": "personal",
      "id": 0
    },
    "configured": null,
    "grants": [],
    "error": "invalid_owner"
  },
  {
    "name": "negative-owner",
    "owner": {
      "kind": "team",
      "id": -1
    },
    "configured": null,
    "grants": [],
    "error": "invalid_owner"
  },
  {
    "name": "invalid-candidate",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "team",
        "id": 0
      }
    ],
    "grants": [],
    "error": "invalid_order"
  },
  {
    "name": "invalid-negative-candidate",
    "owner": {
      "kind": "personal",
      "id": 7
    },
    "configured": [
      {
        "kind": "personal",
        "id": -1
      }
    ],
    "grants": [],
    "error": "invalid_order"
  },
  {
    "name": "team-id-not-personal-wallet",
    "owner": {
      "kind": "team",
      "id": 7
    },
    "configured": [
      {
        "kind": "personal",
        "id": 7
      }
    ],
    "grants": [
      7
    ],
    "error": "invalid_order"
  }
]
```
