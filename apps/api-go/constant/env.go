package constant

var StreamingTimeout int
var DifyDebug bool
var MaxFileDownloadMB int
var StreamScannerMaxBufferMB int
var ForceStreamOption bool
var CountToken bool
var GetMediaToken bool
var GetMediaTokenNotStream bool
var UpdateTask bool
var MaxRequestBodyMB int
var MaxResponseBodyMB int
var AnonymousRequestBodyLimitKB int
var AzureDefaultAPIVersion string
var NotifyLimitCount int
var NotificationLimitDurationMinute int
var GenerateDefaultToken bool
var ErrorLogEnabled bool
var TaskQueryLimit int
var TaskTimeoutMinutes int

const (
	// DefaultTaskQueryLimit bounds the number of unfinished async tasks loaded
	// by one provider-polling pass when the environment is absent or invalid.
	DefaultTaskQueryLimit = 1000
	// MaxTaskQueryLimit prevents a mistaken environment value from turning a
	// bounded polling read into an unbounded in-memory query.
	MaxTaskQueryLimit = 10000
)

// TaskPollingConcurrency caps simultaneous provider polling workers.
var TaskPollingConcurrency int

// SystemTaskHistoryKeep is the number of terminal rows retained per task type.
var SystemTaskHistoryKeep int

// temporary variable for sora patch, will be removed in future
var TaskPricePatches []string

// TrustedRedirectDomains is a list of trusted domains for redirect URL validation.
// Domains support subdomain matching (e.g., "example.com" matches "sub.example.com").
var TrustedRedirectDomains []string
