package assistant_services

import (
	"regexp"
	"strings"
)

var quotedUserText = regexp.MustCompile("(?s)\"[^\"]*\"|'[^']*'|`[^`]*`|“[^”]*”|‘[^’]*’|「[^」]*」|『[^』]*』")

const negativePrefix = `(?:不(?:需要|想要|愿意|允许|可以|能够|想|要|用|需|必|能|可)?|无需|没有必要|别|请勿|勿|拒绝|do\s+not(?:\s+(?:want|need)(?:\s+you)?(?:\s+to)?)?|don't(?:\s+(?:want|need)(?:\s+you)?(?:\s+to)?)?|(?:must|should|can|could|would)\s+not|can't|cannot|shouldn't|mustn't|never|no\s+need(?:\s+to)?|not\s+to)`

var profileNegative = regexp.MustCompile(negativePrefix + `(?:\s*(?:再|先|暂时|现在|帮我|替我|给我|为我|帮忙|请|你|您|to|for|please|a|an|any|the|my|me)\s*)*\s*(?:删除|删掉|移除|清除|清空|忘记|重置|delete|remove|erase|clear|forget|reset)`)
var profileTarget = regexp.MustCompile(`删除|删掉|移除|清除|清空|忘记|重置|delete|remove|erase|clear|forget|reset`)
var negative = regexp.MustCompile(negativePrefix)
var intentClauses = regexp.MustCompile(`[，,。.;；!?！？\n]+`)
var readOnlyIntent = regexp.MustCompile(`(?:^|[，,。.;；!?！？\n])\s*(?:请)?(?:只读(?:验收|检查|测试|模式|操作)?|read[- ]only(?:\s+(?:test|check|audit|validation))?|仅查询|只查询)(?:$|[\s:：，,。.;；!?！？])`)

func containsAny(text string, values ...string) bool {
	for _, value := range values {
		if strings.Contains(text, value) {
			return true
		}
	}
	return false
}
func profileForgetIntent(original string) bool {
	text := strings.ToLower(strings.TrimSpace(strings.ReplaceAll(quotedUserText.ReplaceAllString(original, ""), "’", "'")))
	if text == "" || readOnlyIntent.MatchString(text) || containsAny(text, "如何删除", "怎么删除", "怎样删除", "how to delete", "how can i remove", "how do i remove") {
		return false
	}
	switch strings.Trim(text, " ，,。.;；!?！？\n") {
	case "不要了", "不用了", "不需要了", "算了", "取消", "cancel", "never mind", "nevermind", "no thanks":
		return false
	}
	for _, clause := range intentClauses.Split(text, -1) {
		if profileNegative.MatchString(clause) {
			return false
		}
		if strings.ContainsAny(clause, "、或") || strings.Contains(clause, " or ") {
			n, t := negative.FindStringIndex(clause), profileTarget.FindStringIndex(clause)
			if n != nil && t != nil && n[0] < t[0] {
				return false
			}
		}
	}
	return containsAny(text, "用户画像", "个人画像", "ai画像", "ai profile", "assistant profile", "profile skill", "personalization", "我的画像", "我的标签", "ai标签", "ai 标签", "标签", "tags", "my profile") && containsAny(text, "删除", "删掉", "移除", "清除", "清空", "忘记", "重置", "delete", "remove", "erase", "clear", "forget", "reset") && containsAny(text, "帮我", "请", "我要", "我想", "替我", "给我", "我的", "please", "can you", "could you", "would you", "i want", "i'd like", "my", "mine")
}

var bookingRequest = regexp.MustCompile(`(?i)(?:预约|預約).{0,60}(?:人工|技术支持|技術支持)|\b(?:book|schedule)\b.{0,60}(?:human|technical).{0,20}(?:support|appointment)`)
var bookingNegative = regexp.MustCompile(`(?i)(?:不想|不需要|无需|無需|不要|不用|别|別|勿|不打算|没打算|沒打算|do not want to|don't want to|do not need to|don't need to|no need to|never|cancel).*(?:预约|預約|book|schedule|appointment)`)

func explicitBooking(original string) bool {
	text := strings.ToLower(strings.TrimSpace(quotedUserText.ReplaceAllString(original, "")))
	if bookingNegative.MatchString(text) {
		return false
	}
	switch strings.Trim(text, "。.!！ ") {
	case "取消", "算了", "不用了", "先不用", "暂时不用", "暫時不用", "不要了", "不用人工了", "不需要人工", "cancel", "never mind", "nevermind", "not now":
		return false
	}
	if containsAny(text, "取消预约", "取消預約", "不要预约", "不要預約", "不预约", "不預約", "不用预约", "不用預約", "暂不预约", "暫不預約", "先不预约", "先不預約", "cancel the appointment", "cancel my appointment", "don't book", "do not book") {
		return false
	}
	questionText := text
	if request := bookingRequest.FindStringIndex(text); request != nil {
		if boundary := strings.IndexAny(text[request[1]:], "，,。;；：:\n"); boundary >= 0 {
			questionText = text[:request[1]+boundary]
		}
	}
	if containsAny(questionText, "如何", "怎么", "怎麼", "怎样", "怎樣", "能否", "能不能", "可不可以", "可以预约吗", "可以預約嗎", "可以吗", "可以嗎", "what", "how", "如果", "假如", "假设", "假設", " if ", "是否", "这句话", "這句話", "这三个字", "這三個字", "?", "？") {
		return false
	}
	return bookingRequest.MatchString(text) || containsAny(text, "预约人工", "預約人工", "预约技术支持", "預約技術支持", "book technical support", "book human support", "schedule technical support", "schedule human support", "book a technical support appointment")
}
