/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import fs from 'node:fs/promises'
import path from 'node:path'

const LOCALES_DIR = path.resolve('src/i18n/locales')

function stableStringify(obj) {
  return `${JSON.stringify(obj, null, 2)}\n`
}

const newKeys = {
  en: {
    Guide: 'Guide',
    'View model pricing': 'View model pricing',
    'Browse open-source work': 'Browse open-source work',
    'Read the guide': 'Read the guide',
    'At a glance': 'At a glance',
    'One endpoint': 'One endpoint',
    'OpenAI and Anthropic-compatible routes.':
      'OpenAI and Anthropic-compatible routes.',
    'Clear pricing': 'Clear pricing',
    'Choose the model and group before you spend.':
      'Choose the model and group before you spend.',
    'Human review': 'Human review',
    'Support and access requests stay auditable.':
      'Support and access requests stay auditable.',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      'Use one clear API for your work, connect a client, or explore public open-source challenges.',
    'Cost control': 'Cost control',
    'Financial overview': 'Financial overview',
    Expenses: 'Expenses',
    Profit: 'Profit',
    'Token economy': 'Token economy',
    'External expense': 'External expense',
    'Add expense': 'Add expense',
    'Record expense': 'Record expense',
    'Past 7 days': 'Past 7 days',
    'Past 30 days': 'Past 30 days',
    'Past 90 days': 'Past 90 days',
    'Payment method': 'Payment method',
    'No entries': 'No entries',
    Estimated: 'Estimated',
    'Unpriced requests': 'Unpriced requests',
    'View user': 'View user',
    'User spending': 'User spending',
    'Include revenue': 'Include revenue',
    'Save settings': 'Save settings',
    Reversal: 'Reversal',
    Reverse: 'Reverse',
    'Profit margin': 'Profit margin',
    'Classic chat': 'Classic chat',
    'Modern chat': 'Modern chat',
    'Use classic layout': 'Use classic layout',
    'Use modern layout': 'Use modern layout',
    'New conversation': 'New conversation',
    Examples: 'Examples',
    Capabilities: 'Capabilities',
    Limitations: 'Limitations',
    'Explain an API setup': 'Explain an API setup',
    'Compare live model pricing': 'Compare live model pricing',
    'Draft an access request': 'Draft an access request',
    'Live models and pricing': 'Live models and pricing',
    'Step-by-step setup guidance': 'Step-by-step setup guidance',
    'Confirm sensitive actions yourself': 'Confirm sensitive actions yourself',
    'Permissions still apply': 'Permissions still apply',
    'Never share secrets in chat': 'Never share secrets in chat',
    'Write actions need your confirmation':
      'Write actions need your confirmation',
    'Unable to load data': 'Unable to load data',
    'Append-only ledger': 'Append-only ledger',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.',
    'Unable to load archive': 'Unable to load archive',
    'View L1 recommendation archive': 'View L1 recommendation archive',
    'L1 recommendation archive': 'L1 recommendation archive',
    'No approved recommendation archive yet.':
      'No approved recommendation archive yet.',
    Approved: 'Approved',
    Request: 'Request',
    'Administrator replied': 'Administrator replied',
    'AI recommendation (optional)': 'AI recommendation (optional)',
    'Submit for administrator review': 'Submit for administrator review',
    'Waiting for an administrator': 'Waiting for an administrator',
    'Unable to load your support tasks': 'Unable to load your support tasks',
    'The administrator marked this request resolved.':
      'The administrator marked this request resolved.',
    'Administrator note': 'Administrator note',
    'User skills': 'User skills',
    'Security reviews': 'Security reviews',
    'assistant.security_review': 'Assistant security review',
    'Security audit details': 'Security audit details',
    'Audit data is available to administrators only.':
      'Audit data is available to administrators only.',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.',
    'Protected groups': 'Protected groups',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      'Only groups listed by an enabled rule are included. Rules do not apply globally.',
    'No groups are currently covered by enabled advanced security rules.':
      'No groups are currently covered by enabled advanced security rules.',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.',
    'No protected groups are published yet.':
      'No protected groups are published yet.',
    'Deterministic rule': 'Deterministic rule',
    'All categories': 'All categories',
    'All groups': 'All groups',
    'All decisions': 'All decisions',
    'All sources': 'All sources',
    Violation: 'Violation',
    Clear: 'Clear',
    Reviews: 'Reviews',
    Abuse: 'Abuse',
    Occurred: 'Occurred',
    'Review source': 'Review source',
    'No security audit events match the current filters.':
      'No security audit events match the current filters.',
    'Group warning': 'Group warning',
    'Confirmation {{current}} of {{total}}':
      'Confirmation {{current}} of {{total}}',
    'I understand, continue': 'I understand, continue',
  },
  zh: {
    Guide: '接入指南',
    'View model pricing': '查看模型价格',
    'Browse open-source work': '浏览开源任务',
    'Read the guide': '阅读接入指南',
    'At a glance': '快速了解',
    'One endpoint': '一个接口',
    'OpenAI and Anthropic-compatible routes.':
      '兼容 OpenAI 与 Anthropic 的接口。',
    'Clear pricing': '价格透明',
    'Choose the model and group before you spend.':
      '先选择模型和分组，再开始使用。',
    'Human review': '人工审核',
    'Support and access requests stay auditable.': '支持与访问申请都可追溯。',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      '用一个清晰的 API 完成工作、连接客户端，或探索公开的开源任务。',
    'Cost control': '成本控制',
    'Financial overview': '财务概览',
    Expenses: '支出',
    Profit: '利润',
    'Token economy': 'Token 经济',
    'External expense': '外部支出',
    'Add expense': '添加支出',
    'Record expense': '记录支出',
    'Past 7 days': '近 7 天',
    'Past 30 days': '近 30 天',
    'Past 90 days': '近 90 天',
    'Payment method': '支付方式',
    'No entries': '暂无记录',
    Estimated: '估算',
    'Unpriced requests': '未定价请求',
    'View user': '查看用户',
    'User spending': '用户支出',
    'Include revenue': '计入收入',
    'Save settings': '保存设置',
    Reversal: '冲销',
    Reverse: '冲销',
    'Profit margin': '利润率',
    'Classic chat': '经典对话',
    'Modern chat': '现代对话',
    'Use classic layout': '使用经典布局',
    'Use modern layout': '使用现代布局',
    'New conversation': '新建对话',
    Examples: '示例',
    Capabilities: '可以做什么',
    Limitations: '边界说明',
    'Explain an API setup': '解释 API 配置步骤',
    'Compare live model pricing': '比较实时模型价格',
    'Draft an access request': '起草访问申请',
    'Live models and pricing': '实时模型与价格',
    'Step-by-step setup guidance': '按步骤指导配置',
    'Confirm sensitive actions yourself': '敏感操作由你亲自确认',
    'Permissions still apply': '权限规则仍然生效',
    'Never share secrets in chat': '不要在聊天中发送密钥',
    'Write actions need your confirmation': '写入操作需要你的确认',
    'Unable to load data': '无法加载数据',
    'Append-only ledger': '仅追加记账',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      '账本明细仅包含已持久化的账本事件。请以财务概览的对账收入为准。',
    'Unable to load archive': '无法加载归档',
    'View L1 recommendation archive': '查看 L1 推荐信归档',
    'L1 recommendation archive': 'L1 推荐信归档',
    'No approved recommendation archive yet.': '暂无已批准的推荐信归档。',
    Approved: '已批准',
    Request: '申请',
    'Administrator replied': '管理员已回复',
    'AI recommendation (optional)': 'AI 推荐信（可选）',
    'Submit for administrator review': '提交管理员审核',
    'Waiting for an administrator': '等待管理员处理',
    'Unable to load your support tasks': '无法加载你的客服任务',
    'The administrator marked this request resolved.':
      '管理员已将此申请标记为已解决。',
    'Administrator note': '管理员意见',
    'User skills': '用户技能',
    'Security reviews': '安全巡检',
    'assistant.security_review': '助手安全巡检',
    'Security audit details': '安全审计详情',
    'Audit data is available to administrators only.': '审计数据仅管理员可见。',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      '查看确定性规则和异步 AI 审计结果。此处不会显示提示词、预览、匹配模式或凭证。',
    'Protected groups': '受保护分组',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      '仅启用规则列出的分组会受到保护；规则不会全局生效。',
    'No groups are currently covered by enabled advanced security rules.':
      '当前没有分组受到已启用高级安全规则保护。',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      '只有明确列出的分组会受到高级安全规则保护；规则不会全局生效。',
    'No protected groups are published yet.': '暂未公布受保护分组。',
    'Deterministic rule': '确定性规则',
    'All categories': '全部分类',
    'All groups': '全部分组',
    'All decisions': '全部决策',
    'All sources': '全部来源',
    Violation: '违规',
    Clear: '通过',
    Reviews: '次审计',
    Abuse: '滥用',
    Occurred: '发生时间',
    'Review source': '审计来源',
    'No security audit events match the current filters.':
      '没有符合当前筛选条件的安全审计事件。',
    'Group warning': '分组警告',
    'Confirmation {{current}} of {{total}}': '第 {{current}}/{{total}} 次确认',
    'I understand, continue': '我已了解，继续',
  },
  'zh-TW': {
    Guide: '接入指南',
    'View model pricing': '查看模型價格',
    'Browse open-source work': '瀏覽開源任務',
    'Read the guide': '閱讀接入指南',
    'At a glance': '快速了解',
    'One endpoint': '一個介面',
    'OpenAI and Anthropic-compatible routes.':
      '相容 OpenAI 與 Anthropic 的介面。',
    'Clear pricing': '價格透明',
    'Choose the model and group before you spend.':
      '先選擇模型和分組，再開始使用。',
    'Human review': '人工審核',
    'Support and access requests stay auditable.': '支援與存取申請都可追溯。',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      '用一個清晰的 API 完成工作、連接客戶端，或探索公開的開源任務。',
    'Cost control': '成本控制',
    'Financial overview': '財務概覽',
    Expenses: '支出',
    Profit: '利潤',
    'Token economy': 'Token 經濟',
    'External expense': '外部支出',
    'Add expense': '新增支出',
    'Record expense': '記錄支出',
    'Past 7 days': '近 7 天',
    'Past 30 days': '近 30 天',
    'Past 90 days': '近 90 天',
    'Payment method': '付款方式',
    'No entries': '暫無記錄',
    Estimated: '估算',
    'Unpriced requests': '未定價請求',
    'View user': '查看使用者',
    'User spending': '使用者支出',
    'Include revenue': '計入收入',
    'Save settings': '儲存設定',
    Reversal: '沖銷',
    Reverse: '沖銷',
    'Profit margin': '利潤率',
    'Classic chat': '經典對話',
    'Modern chat': '現代對話',
    'Use classic layout': '使用經典版面',
    'Use modern layout': '使用現代版面',
    'New conversation': '新增對話',
    Examples: '範例',
    Capabilities: '可以做什麼',
    Limitations: '界線說明',
    'Explain an API setup': '解釋 API 設定步驟',
    'Compare live model pricing': '比較即時模型價格',
    'Draft an access request': '起草存取申請',
    'Live models and pricing': '即時模型與價格',
    'Step-by-step setup guidance': '按步驟引導設定',
    'Confirm sensitive actions yourself': '敏感操作由你親自確認',
    'Permissions still apply': '權限規則仍然有效',
    'Never share secrets in chat': '不要在對話中傳送密鑰',
    'Write actions need your confirmation': '寫入操作需要你的確認',
    'Unable to load data': '無法載入資料',
    'Append-only ledger': '僅追加記帳',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      '帳本明細僅包含已持久化的帳本事件。請以財務概覽的對帳收入為準。',
    'Unable to load archive': '無法載入歸檔',
    'View L1 recommendation archive': '查看 L1 推薦信歸檔',
    'L1 recommendation archive': 'L1 推薦信歸檔',
    'No approved recommendation archive yet.': '尚無已核准的推薦信歸檔。',
    Approved: '已核准',
    Request: '申請',
    'Administrator replied': '管理員已回覆',
    'AI recommendation (optional)': 'AI 推薦信（選填）',
    'Submit for administrator review': '提交管理員審核',
    'Waiting for an administrator': '等待管理員處理',
    'Unable to load your support tasks': '無法載入你的客服任務',
    'The administrator marked this request resolved.':
      '管理員已將此申請標記為已解決。',
    'Administrator note': '管理員備註',
    'User skills': '使用者技能',
    'Security reviews': '安全巡檢',
    'assistant.security_review': '助手安全巡檢',
    'Security audit details': '安全稽核詳情',
    'Audit data is available to administrators only.': '稽核資料僅管理員可見。',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      '檢視確定性規則與非同步 AI 稽核結果。此處不會顯示提示文字、預覽、比對模式或憑證。',
    'Protected groups': '受保護分組',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      '僅啟用規則列出的分組會受到保護；規則不會全域套用。',
    'No groups are currently covered by enabled advanced security rules.':
      '目前沒有分組受到已啟用的進階安全規則保護。',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      '只有明確列出的分組會受到進階安全規則保護；規則不會全域套用。',
    'No protected groups are published yet.': '尚未公布受保護分組。',
    'Deterministic rule': '確定性規則',
    'All categories': '全部分類',
    'All groups': '全部分組',
    'All decisions': '全部決策',
    'All sources': '全部來源',
    Violation: '違規',
    Clear: '通過',
    Reviews: '次稽核',
    Abuse: '濫用',
    Occurred: '發生時間',
    'Review source': '稽核來源',
    'No security audit events match the current filters.':
      '沒有符合目前篩選條件的安全稽核事件。',
  },
  fr: {
    Guide: 'Guide',
    'View model pricing': 'Voir les tarifs des modèles',
    'Browse open-source work': 'Parcourir les projets open source',
    'Read the guide': 'Lire le guide',
    'At a glance': 'En bref',
    'One endpoint': 'Un seul endpoint',
    'OpenAI and Anthropic-compatible routes.':
      'Routes compatibles avec OpenAI et Anthropic.',
    'Clear pricing': 'Tarifs clairs',
    'Choose the model and group before you spend.':
      'Choisissez le modèle et le groupe avant de dépenser.',
    'Human review': 'Relecture humaine',
    'Support and access requests stay auditable.':
      'Le support et les demandes d’accès restent auditables.',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      'Utilisez une API claire, connectez un client ou explorez des projets open source.',
    'Cost control': 'Contrôle des coûts',
    'Financial overview': 'Aperçu financier',
    Expenses: 'Dépenses',
    Profit: 'Bénéfice',
    'Token economy': 'Économie des tokens',
    'External expense': 'Dépense externe',
    'Add expense': 'Ajouter une dépense',
    'Record expense': 'Enregistrer la dépense',
    'Past 7 days': '7 derniers jours',
    'Past 30 days': '30 derniers jours',
    'Past 90 days': '90 derniers jours',
    'Payment method': 'Mode de paiement',
    'No entries': 'Aucun enregistrement',
    Estimated: 'Estimé',
    'Unpriced requests': 'Requêtes sans prix',
    'View user': 'Voir l’utilisateur',
    'User spending': 'Dépenses utilisateur',
    'Include revenue': 'Inclure dans les revenus',
    'Save settings': 'Enregistrer les paramètres',
    Reversal: 'Contrepassation',
    Reverse: 'Contrepasser',
    'Profit margin': 'Marge bénéficiaire',
    'Classic chat': 'Chat classique',
    'Modern chat': 'Chat moderne',
    'Use classic layout': 'Utiliser la mise en page classique',
    'Use modern layout': 'Utiliser la mise en page moderne',
    'New conversation': 'Nouvelle conversation',
    Examples: 'Exemples',
    Capabilities: 'Capacités',
    Limitations: 'Limites',
    'Explain an API setup': 'Expliquer une configuration API',
    'Compare live model pricing': 'Comparer les prix des modèles en direct',
    'Draft an access request': 'Rédiger une demande d’accès',
    'Live models and pricing': 'Modèles et tarifs en direct',
    'Step-by-step setup guidance': 'Guides de configuration étape par étape',
    'Confirm sensitive actions yourself':
      'Confirmer vous-même les actions sensibles',
    'Permissions still apply': 'Les permissions restent applicables',
    'Never share secrets in chat': 'Ne partagez jamais de secrets dans le chat',
    'Write actions need your confirmation':
      'Les actions d’écriture nécessitent votre confirmation',
    'Unable to load data': 'Impossible de charger les données',
    'Append-only ledger': 'Grand livre en ajout uniquement',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      'Les écritures se limitent aux événements durables du grand livre. Utilisez la vue financière pour les revenus rapprochés.',
    'Unable to load archive': 'Impossible de charger les archives',
    'View L1 recommendation archive': 'Voir les archives de recommandations L1',
    'L1 recommendation archive': 'Archives de recommandations L1',
    'No approved recommendation archive yet.':
      'Aucune recommandation approuvée archivée.',
    Approved: 'Approuvée',
    Request: 'Demande',
    'Administrator replied': 'Réponse de l’administrateur',
    'AI recommendation (optional)': 'Recommandation IA (facultatif)',
    'Submit for administrator review':
      'Soumettre à l’examen de l’administrateur',
    'Waiting for an administrator': 'En attente d’un administrateur',
    'Unable to load your support tasks':
      'Impossible de charger vos tâches d’assistance',
    'The administrator marked this request resolved.':
      'L’administrateur a marqué cette demande comme résolue.',
    'Administrator note': 'Note de l’administrateur',
    'User skills': 'Compétences utilisateur',
    'Security reviews': 'Revues de sécurité',
    'assistant.security_review': 'Revue de sécurité de l’assistant',
    'Security audit details': 'Détails de l’audit de sécurité',
    'Audit data is available to administrators only.':
      'Les données d’audit sont réservées aux administrateurs.',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      'Consultez les résultats des règles déterministes et des audits IA asynchrones. Les prompts, aperçus, motifs et identifiants ne sont jamais affichés ici.',
    'Protected groups': 'Groupes protégés',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      'Seuls les groupes listés par une règle active sont inclus ; les règles ne sont pas globales.',
    'No groups are currently covered by enabled advanced security rules.':
      'Aucun groupe n’est actuellement couvert par les règles de sécurité avancées actives.',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      'Seuls les groupes explicitement listés sont couverts par les règles avancées ; elles ne sont pas globales.',
    'No protected groups are published yet.': 'Aucun groupe protégé publié.',
    'Deterministic rule': 'Règle déterministe',
    'All categories': 'Toutes les catégories',
    'All groups': 'Tous les groupes',
    'All decisions': 'Toutes les décisions',
    'All sources': 'Toutes les sources',
    Violation: 'Violation',
    Clear: 'Aucun problème',
    Reviews: 'audits',
    Abuse: 'Abus',
    Occurred: 'Date',
    'Review source': 'Source de l’audit',
    'No security audit events match the current filters.':
      'Aucun événement d’audit de sécurité ne correspond aux filtres actuels.',
  },
  ja: {
    Guide: 'ガイド',
    'View model pricing': 'モデル料金を見る',
    'Browse open-source work': 'オープンソースの仕事を見る',
    'Read the guide': 'ガイドを読む',
    'At a glance': '概要',
    'One endpoint': 'ひとつのエンドポイント',
    'OpenAI and Anthropic-compatible routes.':
      'OpenAI と Anthropic に対応したルート。',
    'Clear pricing': '明確な料金',
    'Choose the model and group before you spend.':
      '利用前にモデルとグループを選べます。',
    'Human review': '人による確認',
    'Support and access requests stay auditable.':
      'サポートとアクセス申請を監査可能に保ちます。',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      'ひとつの API で作業し、クライアントを接続し、公開オープンソースの課題を探せます。',
    'Cost control': 'コスト管理',
    'Financial overview': '財務概要',
    Expenses: '支出',
    Profit: '利益',
    'Token economy': 'Token 経済',
    'External expense': '外部支出',
    'Add expense': '支出を追加',
    'Record expense': '支出を記録',
    'Past 7 days': '過去 7 日間',
    'Past 30 days': '過去 30 日間',
    'Past 90 days': '過去 90 日間',
    'Payment method': '支払方法',
    'No entries': '記録なし',
    Estimated: '推定',
    'Unpriced requests': '価格未設定のリクエスト',
    'View user': 'ユーザーを表示',
    'User spending': 'ユーザー支出',
    'Include revenue': '収益に含める',
    'Save settings': '設定を保存',
    Reversal: '取消仕訳',
    Reverse: '取り消す',
    'Profit margin': '利益率',
    'Classic chat': 'クラシックチャット',
    'Modern chat': 'モダンチャット',
    'Use classic layout': 'クラシックレイアウトを使う',
    'Use modern layout': 'モダンレイアウトを使う',
    'New conversation': '新しい会話',
    Examples: '例',
    Capabilities: 'できること',
    Limitations: '制限事項',
    'Explain an API setup': 'API の設定を説明する',
    'Compare live model pricing': '現在のモデル価格を比較する',
    'Draft an access request': 'アクセス申請を下書きする',
    'Live models and pricing': 'ライブモデルと料金',
    'Step-by-step setup guidance': '手順に沿った設定案内',
    'Confirm sensitive actions yourself': '重要な操作は自分で確認する',
    'Permissions still apply': '権限ルールは適用されます',
    'Never share secrets in chat': 'チャットに秘密情報を送らない',
    'Write actions need your confirmation': '書き込み操作には確認が必要です',
    'Unable to load data': 'データを読み込めません',
    'Append-only ledger': '追記専用台帳',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      '台帳明細には永続化された台帳イベントのみが含まれます。照合済み収益は財務概要で確認してください。',
    'Unable to load archive': 'アーカイブを読み込めません',
    'View L1 recommendation archive': 'L1 推薦文アーカイブを表示',
    'L1 recommendation archive': 'L1 推薦文アーカイブ',
    'No approved recommendation archive yet.':
      '承認済みの推薦文アーカイブはありません。',
    Approved: '承認済み',
    Request: '申請',
    'Administrator replied': '管理者からの返信',
    'AI recommendation (optional)': 'AI 推薦文（任意）',
    'Submit for administrator review': '管理者の審査に送信',
    'Waiting for an administrator': '管理者の対応待ち',
    'Unable to load your support tasks': 'サポートタスクを読み込めません',
    'The administrator marked this request resolved.':
      '管理者がこの申請を解決済みにしました。',
    'Administrator note': '管理者メモ',
    'User skills': 'ユーザースキル',
    'Security reviews': 'セキュリティレビュー',
    'assistant.security_review': 'アシスタントのセキュリティレビュー',
    'Security audit details': 'セキュリティ監査の詳細',
    'Audit data is available to administrators only.':
      '監査データは管理者のみ利用できます。',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      '決定論的ルールと非同期 AI 監査の結果を確認します。プロンプト、プレビュー、照合パターン、認証情報は表示されません。',
    'Protected groups': '保護対象グループ',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      '有効なルールに記載されたグループだけが対象です。ルールは全体には適用されません。',
    'No groups are currently covered by enabled advanced security rules.':
      '現在、有効な高度なセキュリティルールの対象グループはありません。',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      '明示的に記載されたグループだけが高度なセキュリティルールの対象です。全体には適用されません。',
    'No protected groups are published yet.':
      '保護対象グループはまだ公開されていません。',
    'Deterministic rule': '決定論的ルール',
    'All categories': 'すべてのカテゴリ',
    'All groups': 'すべてのグループ',
    'All decisions': 'すべての判定',
    'All sources': 'すべてのソース',
    Violation: '違反',
    Clear: '問題なし',
    Reviews: '件の監査',
    Abuse: '悪用',
    Occurred: '発生日時',
    'Review source': '監査ソース',
    'No security audit events match the current filters.':
      '現在のフィルターに一致するセキュリティ監査イベントはありません。',
  },
  ru: {
    Guide: 'Руководство',
    'View model pricing': 'Посмотреть цены моделей',
    'Browse open-source work': 'Открытые проекты',
    'Read the guide': 'Открыть руководство',
    'At a glance': 'Коротко',
    'One endpoint': 'Одна точка доступа',
    'OpenAI and Anthropic-compatible routes.':
      'Маршруты, совместимые с OpenAI и Anthropic.',
    'Clear pricing': 'Понятные цены',
    'Choose the model and group before you spend.':
      'Выберите модель и группу до начала расходов.',
    'Human review': 'Проверка человеком',
    'Support and access requests stay auditable.':
      'Поддержка и запросы доступа остаются проверяемыми.',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      'Используйте единый API, подключайте клиент или изучайте открытые проекты.',
    'Cost control': 'Контроль затрат',
    'Financial overview': 'Финансовый обзор',
    Expenses: 'Расходы',
    Profit: 'Прибыль',
    'Token economy': 'Экономика токенов',
    'External expense': 'Внешний расход',
    'Add expense': 'Добавить расход',
    'Record expense': 'Записать расход',
    'Past 7 days': 'Последние 7 дней',
    'Past 30 days': 'Последние 30 дней',
    'Past 90 days': 'Последние 90 дней',
    'Payment method': 'Способ оплаты',
    'No entries': 'Записей нет',
    Estimated: 'Оценка',
    'Unpriced requests': 'Запросы без цены',
    'View user': 'Открыть пользователя',
    'User spending': 'Расходы пользователя',
    'Include revenue': 'Учитывать в доходе',
    'Save settings': 'Сохранить настройки',
    Reversal: 'Сторно',
    Reverse: 'Сторнировать',
    'Profit margin': 'Маржа',
    'Classic chat': 'Классический чат',
    'Modern chat': 'Современный чат',
    'Use classic layout': 'Использовать классический вид',
    'Use modern layout': 'Использовать современный вид',
    'New conversation': 'Новый разговор',
    Examples: 'Примеры',
    Capabilities: 'Возможности',
    Limitations: 'Ограничения',
    'Explain an API setup': 'Объяснить настройку API',
    'Compare live model pricing': 'Сравнить текущие цены моделей',
    'Draft an access request': 'Подготовить заявку на доступ',
    'Live models and pricing': 'Актуальные модели и цены',
    'Step-by-step setup guidance': 'Пошаговая настройка',
    'Confirm sensitive actions yourself': 'Подтверждайте важные действия сами',
    'Permissions still apply': 'Ограничения доступа сохраняются',
    'Never share secrets in chat': 'Не отправляйте секреты в чат',
    'Write actions need your confirmation':
      'Для изменений нужно ваше подтверждение',
    'Unable to load data': 'Не удалось загрузить данные',
    'Append-only ledger': 'Журнал только для добавления',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      'Записи ограничены устойчивыми событиями журнала. Для сверенных доходов используйте финансовый обзор.',
    'Unable to load archive': 'Не удалось загрузить архив',
    'View L1 recommendation archive': 'Открыть архив рекомендаций L1',
    'L1 recommendation archive': 'Архив рекомендаций L1',
    'No approved recommendation archive yet.':
      'Архивов одобренных рекомендаций пока нет.',
    Approved: 'Одобрено',
    Request: 'Заявка',
    'Administrator replied': 'Ответ администратора',
    'AI recommendation (optional)': 'Рекомендация ИИ (необязательно)',
    'Submit for administrator review': 'Отправить администратору на проверку',
    'Waiting for an administrator': 'Ожидание администратора',
    'Unable to load your support tasks':
      'Не удалось загрузить задачи поддержки',
    'The administrator marked this request resolved.':
      'Администратор отметил этот запрос как решённый.',
    'Administrator note': 'Заметка администратора',
    'User skills': 'Навыки пользователя',
    'Security reviews': 'Проверки безопасности',
    'assistant.security_review': 'Проверка безопасности ассистента',
    'Security audit details': 'Подробности аудита безопасности',
    'Audit data is available to administrators only.':
      'Данные аудита доступны только администраторам.',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      'Результаты детерминированных правил и асинхронных проверок ИИ. Промпты, превью, шаблоны и учётные данные здесь не отображаются.',
    'Protected groups': 'Защищённые группы',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      'Включаются только группы из активных правил; правила не применяются глобально.',
    'No groups are currently covered by enabled advanced security rules.':
      'Сейчас ни одна группа не покрыта активными расширенными правилами безопасности.',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      'Расширенные правила применяются только к явно указанным группам и не глобальны.',
    'No protected groups are published yet.':
      'Защищённые группы пока не опубликованы.',
    'Deterministic rule': 'Детерминированное правило',
    'All categories': 'Все категории',
    'All groups': 'Все группы',
    'All decisions': 'Все решения',
    'All sources': 'Все источники',
    Violation: 'Нарушение',
    Clear: 'Нарушений нет',
    Reviews: 'проверок',
    Abuse: 'Злоупотребление',
    Occurred: 'Время',
    'Review source': 'Источник проверки',
    'No security audit events match the current filters.':
      'Нет событий аудита безопасности, соответствующих текущим фильтрам.',
  },
  vi: {
    Guide: 'Hướng dẫn',
    'View model pricing': 'Xem giá mô hình',
    'Browse open-source work': 'Xem dự án mã nguồn mở',
    'Read the guide': 'Đọc hướng dẫn',
    'At a glance': 'Tổng quan nhanh',
    'One endpoint': 'Một endpoint',
    'OpenAI and Anthropic-compatible routes.':
      'Các route tương thích với OpenAI và Anthropic.',
    'Clear pricing': 'Giá rõ ràng',
    'Choose the model and group before you spend.':
      'Chọn model và nhóm trước khi phát sinh chi phí.',
    'Human review': 'Đánh giá thủ công',
    'Support and access requests stay auditable.':
      'Hỗ trợ và yêu cầu truy cập luôn có thể kiểm tra.',
    'Use one clear API for your work, connect a client, or explore public open-source challenges.':
      'Dùng một API rõ ràng, kết nối client hoặc khám phá dự án mã nguồn mở.',
    'Cost control': 'Kiểm soát chi phí',
    'Financial overview': 'Tổng quan tài chính',
    Expenses: 'Chi phí',
    Profit: 'Lợi nhuận',
    'Token economy': 'Nền kinh tế token',
    'External expense': 'Chi phí bên ngoài',
    'Add expense': 'Thêm chi phí',
    'Record expense': 'Ghi nhận chi phí',
    'Past 7 days': '7 ngày qua',
    'Past 30 days': '30 ngày qua',
    'Past 90 days': '90 ngày qua',
    'Payment method': 'Phương thức thanh toán',
    'No entries': 'Chưa có bản ghi',
    Estimated: 'Ước tính',
    'Unpriced requests': 'Yêu cầu chưa có giá',
    'View user': 'Xem người dùng',
    'User spending': 'Chi tiêu của người dùng',
    'Include revenue': 'Tính vào doanh thu',
    'Save settings': 'Lưu cài đặt',
    Reversal: 'Đảo bút toán',
    Reverse: 'Đảo bút toán',
    'Profit margin': 'Biên lợi nhuận',
    'Classic chat': 'Trò chuyện cổ điển',
    'Modern chat': 'Trò chuyện hiện đại',
    'Use classic layout': 'Dùng bố cục cổ điển',
    'Use modern layout': 'Dùng bố cục hiện đại',
    'New conversation': 'Cuộc trò chuyện mới',
    Examples: 'Ví dụ',
    Capabilities: 'Có thể làm gì',
    Limitations: 'Giới hạn',
    'Explain an API setup': 'Giải thích cách cấu hình API',
    'Compare live model pricing': 'So sánh giá model hiện tại',
    'Draft an access request': 'Soạn yêu cầu cấp quyền',
    'Live models and pricing': 'Model và giá theo thời gian thực',
    'Step-by-step setup guidance': 'Hướng dẫn cài đặt từng bước',
    'Confirm sensitive actions yourself': 'Bạn tự xác nhận thao tác nhạy cảm',
    'Permissions still apply': 'Quyền truy cập vẫn được áp dụng',
    'Never share secrets in chat': 'Không gửi bí mật trong cuộc trò chuyện',
    'Write actions need your confirmation': 'Thao tác ghi cần bạn xác nhận',
    'Unable to load data': 'Không thể tải dữ liệu',
    'Append-only ledger': 'Sổ cái chỉ được ghi thêm',
    'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.':
      'Các mục chỉ gồm sự kiện sổ cái đã được lưu bền vững. Hãy dùng Tổng quan tài chính cho doanh thu đã đối soát.',
    'Unable to load archive': 'Không thể tải kho lưu trữ',
    'View L1 recommendation archive': 'Xem kho lưu trữ đề xuất L1',
    'L1 recommendation archive': 'Kho lưu trữ đề xuất L1',
    'No approved recommendation archive yet.':
      'Chưa có đề xuất nào được phê duyệt trong kho lưu trữ.',
    Approved: 'Đã phê duyệt',
    Request: 'Yêu cầu',
    'Administrator replied': 'Quản trị viên đã phản hồi',
    'AI recommendation (optional)': 'Đề xuất của AI (không bắt buộc)',
    'Submit for administrator review': 'Gửi để quản trị viên xem xét',
    'Waiting for an administrator': 'Đang chờ quản trị viên',
    'Unable to load your support tasks': 'Không thể tải tác vụ hỗ trợ',
    'The administrator marked this request resolved.':
      'Quản trị viên đã đánh dấu yêu cầu này là đã giải quyết.',
    'Administrator note': 'Ghi chú của quản trị viên',
    'User skills': 'Kỹ năng người dùng',
    'Security reviews': 'Đánh giá bảo mật',
    'assistant.security_review': 'Đánh giá bảo mật của trợ lý',
    'Security audit details': 'Chi tiết kiểm tra bảo mật',
    'Audit data is available to administrators only.':
      'Dữ liệu kiểm tra chỉ dành cho quản trị viên.',
    'Review results from deterministic rules and asynchronous AI audits. Prompt text, previews, matcher patterns, and credentials are never shown here.':
      'Xem kết quả từ quy tắc xác định và kiểm tra AI không đồng bộ. Nội dung prompt, bản xem trước, mẫu khớp và thông tin xác thực không được hiển thị.',
    'Protected groups': 'Nhóm được bảo vệ',
    'Only groups listed by an enabled rule are included. Rules do not apply globally.':
      'Chỉ các nhóm được liệt kê trong quy tắc đang bật mới được áp dụng; quy tắc không áp dụng toàn cục.',
    'No groups are currently covered by enabled advanced security rules.':
      'Hiện chưa có nhóm nào được các quy tắc bảo mật nâng cao đang bật bảo vệ.',
    'Only explicitly listed groups are covered by advanced security rules; rules do not apply globally.':
      'Chỉ các nhóm được nêu rõ mới được quy tắc bảo mật nâng cao bảo vệ; quy tắc không áp dụng toàn cục.',
    'No protected groups are published yet.': 'Chưa công bố nhóm được bảo vệ.',
    'Deterministic rule': 'Quy tắc xác định',
    'All categories': 'Tất cả danh mục',
    'All groups': 'Tất cả nhóm',
    'All decisions': 'Tất cả quyết định',
    'All sources': 'Tất cả nguồn',
    Violation: 'Vi phạm',
    Clear: 'Không vi phạm',
    Reviews: 'lượt kiểm tra',
    Abuse: 'Lạm dụng',
    Occurred: 'Thời điểm',
    'Review source': 'Nguồn kiểm tra',
    'No security audit events match the current filters.':
      'Không có sự kiện kiểm tra bảo mật phù hợp với bộ lọc hiện tại.',
  },
}

const drawingTranslations = {
  en: {
    'Drawing studio': 'Drawing studio',
    'Create images through the same safe, group-aware relay used by the API.':
      'Create images through the same safe, group-aware relay used by the API.',
    'Describe an image': 'Describe an image',
    'Describe what you want to see...': 'Describe what you want to see...',
    'Routing group': 'Routing group',
    'Billing follows the selected group configuration.':
      'Billing follows the selected group configuration.',
    'Image model': 'Image model',
    'Size (optional)': 'Size (optional)',
    'Quality (optional)': 'Quality (optional)',
    'Generate image': 'Generate image',
    'Generating...': 'Generating...',
    'Your generated images will appear here.':
      'Your generated images will appear here.',
    'Image catalog unavailable': 'Image catalog unavailable',
    'No image-capable model and routing group is currently available.':
      'No image-capable model and routing group is currently available.',
    'Unable to generate the image': 'Unable to generate the image',
    'No images were returned': 'No images were returned',
    'Ready to generate an image': 'Ready to generate an image',
    'Review the prompt and routing choice before generating.':
      'Review the prompt and routing choice before generating.',
    Prompt: 'Prompt',
    Images: 'Images',
    'Image generated': 'Image generated',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.',
  },
  zh: {
    'Drawing studio': '绘图工作台',
    'Create images through the same safe, group-aware relay used by the API.':
      '通过与 API 相同的安全分组路由创建图片。',
    'Describe an image': '描述图片',
    'Describe what you want to see...': '描述你想看到的内容……',
    'Routing group': '路由分组',
    'Billing follows the selected group configuration.':
      '费用按所选分组配置结算。',
    'Image model': '图片模型',
    'Size (optional)': '尺寸（可选）',
    'Quality (optional)': '质量（可选）',
    'Generate image': '生成图片',
    'Generating...': '生成中……',
    'Your generated images will appear here.': '生成的图片会显示在这里。',
    'Image catalog unavailable': '图片目录不可用',
    'No image-capable model and routing group is currently available.':
      '当前没有可用的图片模型和路由分组。',
    'Unable to generate the image': '无法生成图片',
    'No images were returned': '没有返回图片',
    'Ready to generate an image': '已准备生成图片',
    'Review the prompt and routing choice before generating.':
      '请在生成前检查提示词和路由选择。',
    Prompt: '提示词',
    Images: '图片数量',
    'Image generated': '图片已生成',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      '确认已使用或图片请求失败，请让助手重新准备。',
  },
  'zh-TW': {
    'Drawing studio': '繪圖工作台',
    'Create images through the same safe, group-aware relay used by the API.':
      '透過與 API 相同的安全分組路由建立圖片。',
    'Describe an image': '描述圖片',
    'Describe what you want to see...': '描述你想看到的內容……',
    'Routing group': '路由分組',
    'Billing follows the selected group configuration.':
      '費用依所選分組設定結算。',
    'Image model': '圖片模型',
    'Size (optional)': '尺寸（選填）',
    'Quality (optional)': '品質（選填）',
    'Generate image': '產生圖片',
    'Generating...': '產生中……',
    'Your generated images will appear here.': '產生的圖片會顯示在這裡。',
    'Image catalog unavailable': '圖片目錄無法使用',
    'No image-capable model and routing group is currently available.':
      '目前沒有可用的圖片模型與路由分組。',
    'Unable to generate the image': '無法產生圖片',
    'No images were returned': '沒有回傳圖片',
    'Ready to generate an image': '已準備產生圖片',
    'Review the prompt and routing choice before generating.':
      '產生前請檢查提示詞與路由選擇。',
    Prompt: '提示詞',
    Images: '圖片數量',
    'Image generated': '圖片已產生',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      '確認已使用或圖片請求失敗，請讓助手重新準備。',
  },
  fr: {
    'Drawing studio': 'Atelier de dessin',
    'Create images through the same safe, group-aware relay used by the API.':
      'Créez des images via le même relais sécurisé et sensible aux groupes que l’API.',
    'Describe an image': 'Décrire une image',
    'Describe what you want to see...': 'Décrivez ce que vous voulez voir…',
    'Routing group': 'Groupe de routage',
    'Billing follows the selected group configuration.':
      'La facturation suit la configuration du groupe choisi.',
    'Image model': 'Modèle d’image',
    'Size (optional)': 'Taille (facultatif)',
    'Quality (optional)': 'Qualité (facultatif)',
    'Generate image': 'Générer l’image',
    'Generating...': 'Génération…',
    'Your generated images will appear here.': 'Vos images apparaîtront ici.',
    'Image catalog unavailable': 'Catalogue d’images indisponible',
    'No image-capable model and routing group is currently available.':
      'Aucun modèle d’image ni groupe de routage n’est disponible.',
    'Unable to generate the image': 'Impossible de générer l’image',
    'No images were returned': 'Aucune image reçue',
    'Ready to generate an image': 'Image prête à être générée',
    'Review the prompt and routing choice before generating.':
      'Vérifiez le prompt et le routage avant de générer.',
    Prompt: 'Prompt',
    Images: 'Images',
    'Image generated': 'Image générée',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      'La confirmation a été utilisée ou la demande a échoué. Demandez à l’assistant de la préparer à nouveau.',
  },
  ja: {
    'Drawing studio': '画像スタジオ',
    'Create images through the same safe, group-aware relay used by the API.':
      'API と同じ安全なグループ対応リレーで画像を作成します。',
    'Describe an image': '画像を説明',
    'Describe what you want to see...': '見たいものを説明してください…',
    'Routing group': 'ルーティンググループ',
    'Billing follows the selected group configuration.':
      '料金は選択したグループ設定に従います。',
    'Image model': '画像モデル',
    'Size (optional)': 'サイズ（任意）',
    'Quality (optional)': '品質（任意）',
    'Generate image': '画像を生成',
    'Generating...': '生成中…',
    'Your generated images will appear here.':
      '生成した画像がここに表示されます。',
    'Image catalog unavailable': '画像カタログを利用できません',
    'No image-capable model and routing group is currently available.':
      '利用可能な画像モデルとルーティンググループがありません。',
    'Unable to generate the image': '画像を生成できません',
    'No images were returned': '画像が返されませんでした',
    'Ready to generate an image': '画像を生成する準備ができました',
    'Review the prompt and routing choice before generating.':
      '生成前にプロンプトとルーティングを確認してください。',
    Prompt: 'プロンプト',
    Images: '画像数',
    'Image generated': '画像を生成しました',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      '確認が使用済みか画像リクエストに失敗しました。アシスタントに再準備を依頼してください。',
  },
  ru: {
    'Drawing studio': 'Студия изображений',
    'Create images through the same safe, group-aware relay used by the API.':
      'Создавайте изображения через тот же безопасный групповой релей, что и API.',
    'Describe an image': 'Опишите изображение',
    'Describe what you want to see...': 'Опишите, что хотите увидеть…',
    'Routing group': 'Группа маршрутизации',
    'Billing follows the selected group configuration.':
      'Расчёт выполняется по настройкам выбранной группы.',
    'Image model': 'Модель изображений',
    'Size (optional)': 'Размер (необязательно)',
    'Quality (optional)': 'Качество (необязательно)',
    'Generate image': 'Создать изображение',
    'Generating...': 'Создание…',
    'Your generated images will appear here.':
      'Созданные изображения появятся здесь.',
    'Image catalog unavailable': 'Каталог изображений недоступен',
    'No image-capable model and routing group is currently available.':
      'Нет доступной модели изображений и группы маршрутизации.',
    'Unable to generate the image': 'Не удалось создать изображение',
    'No images were returned': 'Изображения не получены',
    'Ready to generate an image': 'Изображение готово к созданию',
    'Review the prompt and routing choice before generating.':
      'Проверьте запрос и маршрут перед созданием.',
    Prompt: 'Запрос',
    Images: 'Изображения',
    'Image generated': 'Изображение создано',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      'Подтверждение использовано или запрос завершился ошибкой. Попросите ассистента подготовить его снова.',
  },
  vi: {
    'Drawing studio': 'Xưởng tạo ảnh',
    'Create images through the same safe, group-aware relay used by the API.':
      'Tạo ảnh qua cùng relay an toàn, hỗ trợ nhóm như API.',
    'Describe an image': 'Mô tả hình ảnh',
    'Describe what you want to see...': 'Mô tả điều bạn muốn thấy…',
    'Routing group': 'Nhóm định tuyến',
    'Billing follows the selected group configuration.':
      'Chi phí tuân theo cấu hình nhóm đã chọn.',
    'Image model': 'Model hình ảnh',
    'Size (optional)': 'Kích thước (tuỳ chọn)',
    'Quality (optional)': 'Chất lượng (tuỳ chọn)',
    'Generate image': 'Tạo hình ảnh',
    'Generating...': 'Đang tạo…',
    'Your generated images will appear here.':
      'Ảnh được tạo sẽ xuất hiện ở đây.',
    'Image catalog unavailable': 'Danh mục hình ảnh không khả dụng',
    'No image-capable model and routing group is currently available.':
      'Hiện không có model hình ảnh và nhóm định tuyến khả dụng.',
    'Unable to generate the image': 'Không thể tạo hình ảnh',
    'No images were returned': 'Không có hình ảnh được trả về',
    'Ready to generate an image': 'Đã sẵn sàng tạo hình ảnh',
    'Review the prompt and routing choice before generating.':
      'Hãy kiểm tra prompt và lựa chọn định tuyến trước khi tạo.',
    Prompt: 'Prompt',
    Images: 'Số ảnh',
    'Image generated': 'Đã tạo hình ảnh',
    'The confirmation was consumed or the image request failed. Ask the assistant to prepare it again.':
      'Xác nhận đã được dùng hoặc yêu cầu tạo ảnh thất bại. Hãy yêu cầu trợ lý chuẩn bị lại.',
  },
}

for (const [locale, translations] of Object.entries(drawingTranslations)) {
  Object.assign(newKeys[locale], translations)
}

const todoTranslations = {
  en: {
    All: 'All',
    'Challenge reviews': 'Challenge reviews',
    'Bounty notifications': 'Bounty notifications',
    'Developer access': 'Developer access',
    'Account actions': 'Account actions',
    'Security incidents': 'Security incidents',
    'Mark all as read': 'Mark all as read',
    'Failed to load to-dos': 'Failed to load to-dos',
    Loading: 'Loading',
    Notification: 'Notification',
    'Submitted challenge work and account requests will appear here.':
      'Submitted challenge work and account requests will appear here.',
    'No pending to-dos': 'No pending to-dos',
    'Assistant support tasks': 'Assistant support tasks',
    'Pending work': 'Pending work',
    'Resolved history': 'Resolved history',
    'Insights and AI cost': 'Insights and AI cost',
    'support tasks waiting for review': 'support tasks waiting for review',
    'No pending support tasks.': 'No pending support tasks.',
    'Processing note': 'Processing note',
    'Completed at': 'Completed at',
    'Privacy-minimized request': 'Privacy-minimized request',
    'No request details provided.': 'No request details provided.',
    'Complete support task': 'Complete support task',
    'Completing...': 'Completing...',
    'Complete task': 'Complete task',
    Refreshing: 'Refreshing',
    'Action required': 'Action required',
    'Assistant support history and insights':
      'Assistant support history and insights',
    'Unable to load intent insights': 'Unable to load intent insights',
    'Unable to load profile insights': 'Unable to load profile insights',
    'Unable to load first-question insights':
      'Unable to load first-question insights',
    'No first-question data yet': 'No first-question data yet',
    'Top first questions': 'Top first questions',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.',
    'Unable to load AI usage and cost': 'Unable to load AI usage and cost',
    'No recent usage': 'No recent usage',
    'Remaining quota units': 'Remaining quota units',
    'Turn explicit human-support requests into clear next actions.':
      'Turn explicit human-support requests into clear next actions.',
    'AI usage and cost': 'AI usage and cost',
    'All clear': 'All clear',
    'Intent signals': 'Intent signals',
    'No resolved support tasks.': 'No resolved support tasks.',
    'Pending support tasks are unavailable.':
      'Pending support tasks are unavailable.',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      'Privacy-minimized assistant intent counts for the last 30 days.',
    'Privacy-minimized profile signals for the last 30 days.':
      'Privacy-minimized profile signals for the last 30 days.',
    'Resolved support history is unavailable.':
      'Resolved support history is unavailable.',
    'Support task completed': 'Support task completed',
    'Unable to complete support task': 'Unable to complete support task',
    'Unable to load assistant support tasks':
      'Unable to load assistant support tasks',
  },
  zh: {
    All: '全部',
    'Challenge reviews': '挑战审核',
    'Bounty notifications': '悬赏通知',
    'Developer access': '开发者访问',
    'Account actions': '账号操作',
    'Security incidents': '安全事件',
    'Mark all as read': '全部标为已读',
    'Failed to load to-dos': '待办加载失败',
    Loading: '加载中',
    Notification: '通知',
    'Submitted challenge work and account requests will appear here.':
      '已提交的挑战成果和账号申请会显示在这里。',
    'No pending to-dos': '暂无待办',
    'Assistant support tasks': 'AI 客服任务',
    'Pending work': '待处理',
    'Resolved history': '已处理记录',
    'Insights and AI cost': '洞察与 AI 成本',
    'support tasks waiting for review': '个客服任务等待审核',
    'No pending support tasks.': '暂无待处理的客服任务。',
    'Processing note': '处理备注',
    'Completed at': '完成于',
    'Privacy-minimized request': '已做隐私最小化的请求',
    'No request details provided.': '未提供请求详情。',
    'Complete support task': '完成客服任务',
    'Completing...': '完成中...',
    'Complete task': '完成任务',
    Refreshing: '刷新中',
    'Action required': '需要处理',
    'Assistant support history and insights': 'AI 客服历史与洞察',
    'Unable to load intent insights': '无法加载意图洞察',
    'Unable to load profile insights': '无法加载画像洞察',
    'Unable to load first-question insights': '无法加载首轮提问洞察',
    'No first-question data yet': '暂无首轮提问数据',
    'Top first questions': '首轮提问前十',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      '最近 30 天按真实用户首轮提问统计的隐私最小化数据。',
    'Unable to load AI usage and cost': '无法加载 AI 用量与成本',
    'No recent usage': '暂无近期用量',
    'Remaining quota units': '剩余额度单位',
    'Turn explicit human-support requests into clear next actions.':
      '将明确的人工客服请求转成清晰的下一步。',
    'AI usage and cost': 'AI 用量与成本',
    'All clear': '全部处理完毕',
    'Intent signals': '意图信号',
    'No resolved support tasks.': '暂无已处理的客服任务。',
    'Pending support tasks are unavailable.': '暂时无法加载待处理的客服任务。',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      '最近 30 天的隐私最小化客服意图统计。',
    'Privacy-minimized profile signals for the last 30 days.':
      '最近 30 天的隐私最小化用户画像信号。',
    'Resolved support history is unavailable.': '暂时无法加载客服处理记录。',
    'Support task completed': '客服任务已完成',
    'Unable to complete support task': '无法完成客服任务',
    'Unable to load assistant support tasks': '无法加载 AI 客服任务',
  },
  'zh-TW': {
    All: '全部',
    'Challenge reviews': '挑戰審核',
    'Bounty notifications': '懸賞通知',
    'Developer access': '開發者存取',
    'Account actions': '帳號操作',
    'Security incidents': '安全事件',
    'Mark all as read': '全部標為已讀',
    'Failed to load to-dos': '待辦載入失敗',
    Loading: '載入中',
    Notification: '通知',
    'Submitted challenge work and account requests will appear here.':
      '已提交的挑戰成果和帳號申請會顯示在這裡。',
    'No pending to-dos': '暫無待辦',
    'Assistant support tasks': 'AI 客服任務',
    'Pending work': '待處理',
    'Resolved history': '已處理記錄',
    'Insights and AI cost': '洞察與 AI 成本',
    'support tasks waiting for review': '個客服任務等待審核',
    'No pending support tasks.': '暫無待處理的客服任務。',
    'Processing note': '處理備註',
    'Completed at': '完成於',
    'Privacy-minimized request': '已做隱私最小化的請求',
    'No request details provided.': '未提供請求詳情。',
    'Complete support task': '完成客服任務',
    'Completing...': '完成中...',
    'Complete task': '完成任務',
    Refreshing: '重新整理中',
    'Action required': '需要處理',
    'Assistant support history and insights': 'AI 客服歷史與洞察',
    'Unable to load intent insights': '無法載入意圖洞察',
    'Unable to load profile insights': '無法載入使用者輪廓洞察',
    'Unable to load first-question insights': '無法載入首輪提問洞察',
    'No first-question data yet': '暫無首輪提問資料',
    'Top first questions': '首輪提問前十',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      '最近 30 天按真實使用者首輪提問統計的隱私最小化資料。',
    'Unable to load AI usage and cost': '無法載入 AI 用量與成本',
    'No recent usage': '暫無近期用量',
    'Remaining quota units': '剩餘額度單位',
    'Turn explicit human-support requests into clear next actions.':
      '將明確的人工客服請求轉成清晰的下一步。',
    'AI usage and cost': 'AI 用量與成本',
    'All clear': '全部處理完畢',
    'Intent signals': '意圖訊號',
    'No resolved support tasks.': '暫無已處理的客服任務。',
    'Pending support tasks are unavailable.': '暫時無法載入待處理的客服任務。',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      '最近 30 天的隱私最小化客服意圖統計。',
    'Privacy-minimized profile signals for the last 30 days.':
      '最近 30 天的隱私最小化使用者輪廓訊號。',
    'Resolved support history is unavailable.': '暫時無法載入客服處理記錄。',
    'Support task completed': '客服任務已完成',
    'Unable to complete support task': '無法完成客服任務',
    'Unable to load assistant support tasks': '無法載入 AI 客服任務',
  },
  fr: {
    All: 'Tout',
    'Challenge reviews': 'Révisions des défis',
    'Bounty notifications': 'Notifications de primes',
    'Developer access': 'Accès développeur',
    'Account actions': 'Actions sur le compte',
    'Security incidents': 'Incidents de sécurité',
    'Mark all as read': 'Tout marquer comme lu',
    'Failed to load to-dos': 'Échec du chargement des tâches',
    Loading: 'Chargement',
    Notification: 'Notification',
    'Submitted challenge work and account requests will appear here.':
      'Les travaux de défi soumis et les demandes de compte apparaîtront ici.',
    'No pending to-dos': 'Aucune tâche en attente',
    'Assistant support tasks': 'Tâches du support IA',
    'Pending work': 'À traiter',
    'Resolved history': 'Historique traité',
    'Insights and AI cost': 'Analyses et coût de l’IA',
    'support tasks waiting for review': 'tâches de support en attente de revue',
    'No pending support tasks.': 'Aucune tâche de support en attente.',
    'Processing note': 'Note de traitement',
    'Completed at': 'Terminé le',
    'Privacy-minimized request': 'Demande minimisée pour la confidentialité',
    'No request details provided.': 'Aucun détail de demande fourni.',
    'Complete support task': 'Terminer la tâche de support',
    'Completing...': 'Finalisation...',
    'Complete task': 'Terminer la tâche',
    Refreshing: 'Actualisation',
    'Action required': 'Action requise',
    'Assistant support history and insights':
      'Historique et analyses du support IA',
    'Unable to load intent insights': 'Impossible de charger les intentions',
    'Unable to load profile insights': 'Impossible de charger les profils',
    'Unable to load first-question insights':
      'Impossible de charger les premières questions',
    'No first-question data yet': 'Aucune première question pour le moment',
    'Top first questions': 'Top 10 des premières questions',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      'Premières questions réelles, minimisées pour la confidentialité, comptées au premier tour sur les 30 derniers jours.',
    'Unable to load AI usage and cost':
      'Impossible de charger l’usage et le coût IA',
    'No recent usage': 'Aucun usage récent',
    'Remaining quota units': 'Unités de quota restantes',
    'Turn explicit human-support requests into clear next actions.':
      'Transformez les demandes explicites au support humain en prochaines étapes claires.',
    'AI usage and cost': 'Utilisation et coût de l’IA',
    'All clear': 'Tout est traité',
    'Intent signals': 'Signaux d’intention',
    'No resolved support tasks.': 'Aucune tâche de support traitée.',
    'Pending support tasks are unavailable.':
      'Les tâches de support en attente sont indisponibles.',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      'Comptage des intentions du support, minimisé pour la confidentialité, sur 30 jours.',
    'Privacy-minimized profile signals for the last 30 days.':
      'Signaux de profil, minimisés pour la confidentialité, sur 30 jours.',
    'Resolved support history is unavailable.':
      'L’historique du support traité est indisponible.',
    'Support task completed': 'Tâche de support terminée',
    'Unable to complete support task':
      'Impossible de terminer la tâche de support',
    'Unable to load assistant support tasks':
      'Impossible de charger les tâches du support IA',
  },
  ja: {
    All: 'すべて',
    'Challenge reviews': 'チャレンジ審査',
    'Bounty notifications': '報奨金通知',
    'Developer access': '開発者アクセス',
    'Account actions': 'アカウント操作',
    'Security incidents': 'セキュリティインシデント',
    'Mark all as read': 'すべて既読にする',
    'Failed to load to-dos': '対応待ちを読み込めませんでした',
    Loading: '読み込み中',
    Notification: '通知',
    'Submitted challenge work and account requests will appear here.':
      '提出されたチャレンジ成果とアカウント申請がここに表示されます。',
    'No pending to-dos': '対応待ちはありません',
    'Assistant support tasks': 'AI サポートタスク',
    'Pending work': '対応待ち',
    'Resolved history': '対応済み履歴',
    'Insights and AI cost': '分析と AI コスト',
    'support tasks waiting for review': '件のサポートタスクが確認待ちです',
    'No pending support tasks.': '対応待ちのサポートタスクはありません。',
    'Processing note': '処理メモ',
    'Completed at': '完了日時',
    'Privacy-minimized request': 'プライバシー最小化済みの依頼',
    'No request details provided.': '依頼の詳細はありません。',
    'Complete support task': 'サポートタスクを完了',
    'Completing...': '完了処理中...',
    'Complete task': 'タスクを完了',
    Refreshing: '更新中',
    'Action required': '対応が必要',
    'Assistant support history and insights': 'AI サポート履歴と分析',
    'Unable to load intent insights': '意図分析を読み込めません',
    'Unable to load profile insights': 'プロフィール分析を読み込めません',
    'Unable to load first-question insights':
      '最初の質問の分析を読み込めません',
    'No first-question data yet': '最初の質問データはまだありません',
    'Top first questions': '最初の質問トップ10',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      '過去30日間の実ユーザーの最初の質問を、初回ターンからプライバシー最小化して集計しています。',
    'Unable to load AI usage and cost': 'AI 使用量とコストを読み込めません',
    'No recent usage': '最近の使用履歴はありません',
    'Remaining quota units': '残りのクォータ単位',
    'Turn explicit human-support requests into clear next actions.':
      '明確な有人サポート依頼を次の行動に整理します。',
    'AI usage and cost': 'AI の使用量とコスト',
    'All clear': 'すべて対応済み',
    'Intent signals': '意図シグナル',
    'No resolved support tasks.': '対応済みのサポートタスクはありません。',
    'Pending support tasks are unavailable.':
      '対応待ちのサポートタスクを読み込めません。',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      '過去 30 日間のプライバシー最小化済みサポート意図数。',
    'Privacy-minimized profile signals for the last 30 days.':
      '過去 30 日間のプライバシー最小化済みプロフィールシグナル。',
    'Resolved support history is unavailable.':
      '対応済みサポート履歴を読み込めません。',
    'Support task completed': 'サポートタスクを完了しました',
    'Unable to complete support task': 'サポートタスクを完了できません',
    'Unable to load assistant support tasks':
      'AI サポートタスクを読み込めません',
  },
  ru: {
    All: 'Все',
    'Challenge reviews': 'Проверка заданий',
    'Bounty notifications': 'Уведомления о наградах',
    'Developer access': 'Доступ разработчика',
    'Account actions': 'Действия с аккаунтом',
    'Security incidents': 'Инциденты безопасности',
    'Mark all as read': 'Отметить всё прочитанным',
    'Failed to load to-dos': 'Не удалось загрузить задачи',
    Loading: 'Загрузка',
    Notification: 'Уведомление',
    'Submitted challenge work and account requests will appear here.':
      'Здесь появятся отправленные решения заданий и запросы аккаунта.',
    'No pending to-dos': 'Нет ожидающих задач',
    'Assistant support tasks': 'Задачи поддержки ИИ',
    'Pending work': 'Ожидающие задачи',
    'Resolved history': 'История решённых задач',
    'Insights and AI cost': 'Аналитика и расходы на ИИ',
    'support tasks waiting for review': 'задач поддержки ожидают проверки',
    'No pending support tasks.': 'Нет ожидающих задач поддержки.',
    'Processing note': 'Заметка обработки',
    'Completed at': 'Завершено',
    'Privacy-minimized request': 'Запрос с минимизацией данных',
    'No request details provided.': 'Подробности запроса не указаны.',
    'Complete support task': 'Завершить задачу поддержки',
    'Completing...': 'Завершение...',
    'Complete task': 'Завершить задачу',
    Refreshing: 'Обновление',
    'Action required': 'Требуется действие',
    'Assistant support history and insights':
      'История и аналитика поддержки ИИ',
    'Unable to load intent insights':
      'Не удалось загрузить аналитику намерений',
    'Unable to load profile insights':
      'Не удалось загрузить аналитику профилей',
    'Unable to load first-question insights':
      'Не удалось загрузить аналитику первых вопросов',
    'No first-question data yet': 'Данных первых вопросов пока нет',
    'Top first questions': 'Топ-10 первых вопросов',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      'Первые вопросы реальных пользователей с минимизацией данных за последние 30 дней.',
    'Unable to load AI usage and cost':
      'Не удалось загрузить расходы и использование ИИ',
    'No recent usage': 'Недавнего использования нет',
    'Remaining quota units': 'Оставшиеся единицы квоты',
    'Turn explicit human-support requests into clear next actions.':
      'Преобразуйте явные запросы к специалисту в понятные следующие шаги.',
    'AI usage and cost': 'Использование и расходы на ИИ',
    'All clear': 'Всё обработано',
    'Intent signals': 'Сигналы намерений',
    'No resolved support tasks.': 'Обработанных задач поддержки нет.',
    'Pending support tasks are unavailable.':
      'Ожидающие задачи поддержки недоступны.',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      'Количество намерений поддержки с минимизацией данных за последние 30 дней.',
    'Privacy-minimized profile signals for the last 30 days.':
      'Сигналы профиля с минимизацией данных за последние 30 дней.',
    'Resolved support history is unavailable.':
      'История обработанных обращений поддержки недоступна.',
    'Support task completed': 'Задача поддержки выполнена',
    'Unable to complete support task': 'Не удалось завершить задачу поддержки',
    'Unable to load assistant support tasks':
      'Не удалось загрузить задачи поддержки ИИ',
  },
  vi: {
    All: 'Tất cả',
    'Challenge reviews': 'Duyệt thử thách',
    'Bounty notifications': 'Thông báo tiền thưởng',
    'Developer access': 'Quyền nhà phát triển',
    'Account actions': 'Thao tác tài khoản',
    'Security incidents': 'Sự cố bảo mật',
    'Mark all as read': 'Đánh dấu tất cả đã đọc',
    'Failed to load to-dos': 'Không thể tải việc cần làm',
    Loading: 'Đang tải',
    Notification: 'Thông báo',
    'Submitted challenge work and account requests will appear here.':
      'Bài làm thử thách và yêu cầu tài khoản đã gửi sẽ xuất hiện tại đây.',
    'No pending to-dos': 'Không có việc đang chờ',
    'Assistant support tasks': 'Tác vụ hỗ trợ AI',
    'Pending work': 'Việc đang chờ',
    'Resolved history': 'Lịch sử đã xử lý',
    'Insights and AI cost': 'Thông tin và chi phí AI',
    'support tasks waiting for review': 'tác vụ hỗ trợ đang chờ duyệt',
    'No pending support tasks.': 'Không có tác vụ hỗ trợ đang chờ.',
    'Processing note': 'Ghi chú xử lý',
    'Completed at': 'Hoàn tất lúc',
    'Privacy-minimized request': 'Yêu cầu đã tối giản dữ liệu riêng tư',
    'No request details provided.': 'Chưa có chi tiết yêu cầu.',
    'Complete support task': 'Hoàn tất tác vụ hỗ trợ',
    'Completing...': 'Đang hoàn tất...',
    'Complete task': 'Hoàn tất tác vụ',
    Refreshing: 'Đang làm mới',
    'Action required': 'Cần xử lý',
    'Assistant support history and insights': 'Lịch sử và thông tin hỗ trợ AI',
    'Unable to load intent insights': 'Không thể tải thông tin ý định',
    'Unable to load profile insights': 'Không thể tải thông tin hồ sơ',
    'Unable to load first-question insights':
      'Không thể tải thông tin câu hỏi đầu tiên',
    'No first-question data yet': 'Chưa có dữ liệu câu hỏi đầu tiên',
    'Top first questions': 'Top 10 câu hỏi đầu tiên',
    'Privacy-minimized real-user first questions counted from the first turn in the last 30 days.':
      'Câu hỏi đầu tiên của người dùng thật, tối giản dữ liệu riêng tư, được đếm từ lượt đầu trong 30 ngày qua.',
    'Unable to load AI usage and cost': 'Không thể tải mức dùng và chi phí AI',
    'No recent usage': 'Chưa có mức dùng gần đây',
    'Remaining quota units': 'Đơn vị hạn mức còn lại',
    'Turn explicit human-support requests into clear next actions.':
      'Chuyển yêu cầu hỗ trợ con người rõ ràng thành các bước tiếp theo.',
    'AI usage and cost': 'Mức dùng và chi phí AI',
    'All clear': 'Đã xử lý xong',
    'Intent signals': 'Tín hiệu ý định',
    'No resolved support tasks.': 'Không có tác vụ hỗ trợ đã xử lý.',
    'Pending support tasks are unavailable.':
      'Không thể tải tác vụ hỗ trợ đang chờ.',
    'Privacy-minimized assistant intent counts for the last 30 days.':
      'Số lượng ý định hỗ trợ đã tối giản dữ liệu riêng tư trong 30 ngày qua.',
    'Privacy-minimized profile signals for the last 30 days.':
      'Tín hiệu hồ sơ đã tối giản dữ liệu riêng tư trong 30 ngày qua.',
    'Resolved support history is unavailable.':
      'Không thể tải lịch sử hỗ trợ đã xử lý.',
    'Support task completed': 'Đã hoàn tất tác vụ hỗ trợ',
    'Unable to complete support task': 'Không thể hoàn tất tác vụ hỗ trợ',
    'Unable to load assistant support tasks': 'Không thể tải tác vụ hỗ trợ AI',
  },
}

const discountTranslations = {
  en: {
    'Discount Codes': 'Discount Codes',
    'Create discount code': 'Create discount code',
    'Edit discount code': 'Edit discount code',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      'Manage percentage discounts for checkout. Codes are validated and applied by the server.',
    'Filter by code or name...': 'Filter by code or name...',
    Code: 'Code',
    Discount: 'Discount',
    Used: 'Used',
    Expires: 'Expires',
    'Unable to save discount code': 'Unable to save discount code',
    'Discount code saved': 'Discount code saved',
    'Unable to update discount code': 'Unable to update discount code',
    'Unable to delete discount code': 'Unable to delete discount code',
    'Discount code deleted': 'Discount code deleted',
    'No discount codes': 'No discount codes',
    'Delete this discount code?': 'Delete this discount code?',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      'Set a percentage discount. The server checks dates and minimum amount at checkout.',
    'Discount percent': 'Discount percent',
    'Minimum amount': 'Minimum amount',
    Starts: 'Starts',
    Apply: 'Apply',
    'Enter your discount code': 'Enter your discount code',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      'A valid discount code is applied at checkout and cannot be combined with another code.',
    'Discount applied: {{percent}}% off': 'Discount applied: {{percent}}% off',
  },
  zh: {
    'Discount Codes': '优惠码',
    'Create discount code': '创建优惠码',
    'Edit discount code': '编辑优惠码',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      '管理结算时使用的百分比折扣。优惠码由服务器校验并应用。',
    'Filter by code or name...': '按代码或名称筛选…',
    Code: '代码',
    Discount: '折扣',
    Used: '已使用',
    Expires: '过期时间',
    'Unable to save discount code': '无法保存优惠码',
    'Discount code saved': '优惠码已保存',
    'Unable to update discount code': '无法更新优惠码',
    'Unable to delete discount code': '无法删除优惠码',
    'Discount code deleted': '优惠码已删除',
    'No discount codes': '暂无优惠码',
    'Delete this discount code?': '删除这个优惠码？',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      '设置百分比折扣。服务器会在结算时检查有效期和最低金额。',
    'Discount percent': '折扣百分比',
    'Minimum amount': '最低金额',
    Starts: '生效时间',
    Apply: '应用',
    'Enter your discount code': '输入优惠码',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      '有效优惠码会在结算时应用，且不能与其他优惠码叠加。',
    'Discount applied: {{percent}}% off': '已应用 {{percent}}% 折扣',
  },
  'zh-TW': {
    'Discount Codes': '優惠碼',
    'Create discount code': '建立優惠碼',
    'Edit discount code': '編輯優惠碼',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      '管理結帳時使用的百分比折扣。優惠碼由伺服器驗證並套用。',
    'Filter by code or name...': '按代碼或名稱篩選…',
    Code: '代碼',
    Discount: '折扣',
    Used: '已使用',
    Expires: '到期時間',
    'Unable to save discount code': '無法儲存優惠碼',
    'Discount code saved': '優惠碼已儲存',
    'Unable to update discount code': '無法更新優惠碼',
    'Unable to delete discount code': '無法刪除優惠碼',
    'Discount code deleted': '優惠碼已刪除',
    'No discount codes': '暫無優惠碼',
    'Delete this discount code?': '刪除此優惠碼？',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      '設定百分比折扣。伺服器會在結帳時檢查有效期與最低金額。',
    'Discount percent': '折扣百分比',
    'Minimum amount': '最低金額',
    Starts: '生效時間',
    Apply: '套用',
    'Enter your discount code': '輸入優惠碼',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      '有效優惠碼會在結帳時套用，且不能與其他優惠碼疊加。',
    'Discount applied: {{percent}}% off': '已套用 {{percent}}% 折扣',
  },
  fr: {
    'Discount Codes': 'Codes promotionnels',
    'Create discount code': 'Créer un code promotionnel',
    'Edit discount code': 'Modifier le code promotionnel',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      'Gérez les remises en pourcentage au paiement. Les codes sont validés et appliqués par le serveur.',
    'Filter by code or name...': 'Filtrer par code ou nom…',
    Code: 'Code',
    Discount: 'Remise',
    Used: 'Utilisé',
    Expires: 'Expire',
    'Unable to save discount code': 'Impossible d’enregistrer le code',
    'Discount code saved': 'Code promotionnel enregistré',
    'Unable to update discount code': 'Impossible de modifier le code',
    'Unable to delete discount code': 'Impossible de supprimer le code',
    'Discount code deleted': 'Code promotionnel supprimé',
    'No discount codes': 'Aucun code promotionnel',
    'Delete this discount code?': 'Supprimer ce code promotionnel ?',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      'Définissez une remise en pourcentage. Le serveur vérifie les dates et le montant minimal au paiement.',
    'Discount percent': 'Pourcentage de remise',
    'Minimum amount': 'Montant minimal',
    Starts: 'Début',
    Apply: 'Appliquer',
    'Enter your discount code': 'Saisissez votre code promotionnel',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      'Un code valide est appliqué au paiement et ne peut pas être combiné avec un autre code.',
    'Discount applied: {{percent}}% off': 'Remise appliquée : {{percent}} %',
  },
  ja: {
    'Discount Codes': '割引コード',
    'Create discount code': '割引コードを作成',
    'Edit discount code': '割引コードを編集',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      '決済時のパーセント割引を管理します。コードはサーバーで検証・適用されます。',
    'Filter by code or name...': 'コードまたは名前で絞り込み…',
    Code: 'コード',
    Discount: '割引',
    Used: '使用数',
    Expires: '有効期限',
    'Unable to save discount code': '割引コードを保存できません',
    'Discount code saved': '割引コードを保存しました',
    'Unable to update discount code': '割引コードを更新できません',
    'Unable to delete discount code': '割引コードを削除できません',
    'Discount code deleted': '割引コードを削除しました',
    'No discount codes': '割引コードはありません',
    'Delete this discount code?': 'この割引コードを削除しますか？',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      'パーセント割引を設定します。決済時にサーバーが期間と最低金額を確認します。',
    'Discount percent': '割引率',
    'Minimum amount': '最低金額',
    Starts: '開始',
    Apply: '適用',
    'Enter your discount code': '割引コードを入力',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      '有効な割引コードは決済時に適用され、他のコードとは併用できません。',
    'Discount applied: {{percent}}% off':
      '割引を適用しました：{{percent}}% オフ',
  },
  ru: {
    'Discount Codes': 'Коды скидок',
    'Create discount code': 'Создать код скидки',
    'Edit discount code': 'Изменить код скидки',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      'Управляйте процентными скидками при оплате. Коды проверяются и применяются сервером.',
    'Filter by code or name...': 'Фильтр по коду или названию…',
    Code: 'Код',
    Discount: 'Скидка',
    Used: 'Использован',
    Expires: 'Истекает',
    'Unable to save discount code': 'Не удалось сохранить код скидки',
    'Discount code saved': 'Код скидки сохранён',
    'Unable to update discount code': 'Не удалось обновить код скидки',
    'Unable to delete discount code': 'Не удалось удалить код скидки',
    'Discount code deleted': 'Код скидки удалён',
    'No discount codes': 'Кодов скидок нет',
    'Delete this discount code?': 'Удалить этот код скидки?',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      'Задайте процентную скидку. Сервер проверит даты и минимальную сумму при оплате.',
    'Discount percent': 'Процент скидки',
    'Minimum amount': 'Минимальная сумма',
    Starts: 'Начало',
    Apply: 'Применить',
    'Enter your discount code': 'Введите код скидки',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      'Действующий код применяется при оплате и не сочетается с другим кодом.',
    'Discount applied: {{percent}}% off': 'Скидка применена: {{percent}}%',
  },
  vi: {
    'Discount Codes': 'Mã giảm giá',
    'Create discount code': 'Tạo mã giảm giá',
    'Edit discount code': 'Chỉnh sửa mã giảm giá',
    'Manage percentage discounts for checkout. Codes are validated and applied by the server.':
      'Quản lý giảm giá theo phần trăm khi thanh toán. Mã được máy chủ xác thực và áp dụng.',
    'Filter by code or name...': 'Lọc theo mã hoặc tên…',
    Code: 'Mã',
    Discount: 'Giảm giá',
    Used: 'Đã dùng',
    Expires: 'Hết hạn',
    'Unable to save discount code': 'Không thể lưu mã giảm giá',
    'Discount code saved': 'Đã lưu mã giảm giá',
    'Unable to update discount code': 'Không thể cập nhật mã giảm giá',
    'Unable to delete discount code': 'Không thể xóa mã giảm giá',
    'Discount code deleted': 'Đã xóa mã giảm giá',
    'No discount codes': 'Chưa có mã giảm giá',
    'Delete this discount code?': 'Xóa mã giảm giá này?',
    'Set a percentage discount. The server checks dates and minimum amount at checkout.':
      'Đặt mức giảm theo phần trăm. Máy chủ kiểm tra ngày và số tiền tối thiểu khi thanh toán.',
    'Discount percent': 'Phần trăm giảm',
    'Minimum amount': 'Số tiền tối thiểu',
    Starts: 'Bắt đầu',
    Apply: 'Áp dụng',
    'Enter your discount code': 'Nhập mã giảm giá',
    'A valid discount code is applied at checkout and cannot be combined with another code.':
      'Mã hợp lệ được áp dụng khi thanh toán và không thể dùng cùng mã khác.',
    'Discount applied: {{percent}}% off': 'Đã áp dụng giảm {{percent}}%',
  },
}

const regionPolicyTranslations = {
  en: {
    'Regional access policy': 'Regional access policy',
    'Blocked country codes': 'Blocked country codes',
    'Require the edge policy to check blocked countries before requests reach the application.':
      'Require the edge policy to check blocked countries before requests reach the application.',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.',
  },
  zh: {
    'Regional access policy': '地域访问限制',
    'Blocked country codes': '阻止的国家/地区代码',
    'Require the edge policy to check blocked countries before requests reach the application.':
      '要求边缘策略在请求进入应用前检查被阻止的国家或地区。',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      '使用逗号分隔的两位 ISO 代码，例如 CN,US。关闭此策略即可允许所有地区。',
  },
  'zh-TW': {
    'Regional access policy': '地區存取限制',
    'Blocked country codes': '封鎖的國家／地區代碼',
    'Require the edge policy to check blocked countries before requests reach the application.':
      '要求邊緣策略在請求進入應用程式前檢查封鎖的國家或地區。',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      '使用逗號分隔的兩位 ISO 代碼，例如 CN,US。關閉此策略即可允許所有地區。',
  },
  fr: {
    'Regional access policy': 'Politique d’accès régional',
    'Blocked country codes': 'Codes des pays bloqués',
    'Require the edge policy to check blocked countries before requests reach the application.':
      'Demander à la politique en périphérie de vérifier les pays bloqués avant l’application.',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      'Codes ISO à deux lettres séparés par des virgules, par exemple CN,US. Désactivez la politique pour autoriser toutes les régions.',
  },
  ja: {
    'Regional access policy': '地域アクセス制限',
    'Blocked country codes': 'ブロックする国コード',
    'Require the edge policy to check blocked countries before requests reach the application.':
      'アプリケーションに到達する前にエッジポリシーでブロック対象国を確認します。',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      'CN,US のように2文字の ISO コードをカンマ区切りで指定します。無効にすると全地域を許可します。',
  },
  ru: {
    'Regional access policy': 'Региональная политика доступа',
    'Blocked country codes': 'Коды заблокированных стран',
    'Require the edge policy to check blocked countries before requests reach the application.':
      'Проверять заблокированные страны на границе до передачи запроса приложению.',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      'Двухбуквенные ISO-коды через запятую, например CN,US. Отключите политику, чтобы разрешить все регионы.',
  },
  vi: {
    'Regional access policy': 'Chính sách truy cập theo khu vực',
    'Blocked country codes': 'Mã quốc gia bị chặn',
    'Require the edge policy to check blocked countries before requests reach the application.':
      'Yêu cầu chính sách biên kiểm tra quốc gia bị chặn trước khi yêu cầu đến ứng dụng.',
    'Comma-separated two-letter ISO codes, for example CN,US. Disable the policy to allow every region.':
      'Nhập mã ISO hai chữ cái cách nhau bằng dấu phẩy, ví dụ CN,US. Tắt chính sách để cho phép mọi khu vực.',
  },
}

const advancedSecurityTranslations = {
  en: {
    'Each advanced security rule must include at least one explicit group.':
      'Each advanced security rule must include at least one explicit group.',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      'Every rule must list the API group or groups it applies to; rules never apply globally.',
  },
  zh: {
    'Each advanced security rule must include at least one explicit group.':
      '每条高级安全规则至少要指定一个明确分组。',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      '规则分组必须是非空的明确名称，长度不超过 64 个字符；不允许使用通配符分组。',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      '每条规则都必须列出它适用的 API 分组；规则不会全局生效。',
  },
  'zh-TW': {
    'Each advanced security rule must include at least one explicit group.':
      '每條進階安全規則至少要指定一個明確分組。',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      '規則分組必須是非空的明確名稱，長度不超過 64 個字元；不允許使用萬用字元分組。',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      '每條規則都必須列出適用的 API 分組；規則不會全域生效。',
  },
  fr: {
    'Each advanced security rule must include at least one explicit group.':
      'Chaque règle de sécurité avancée doit spécifier au moins un groupe explicite.',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      'Les groupes doivent être des noms explicites non vides de 64 caractères maximum ; les groupes génériques sont interdits.',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      'Chaque règle doit indiquer les groupes API auxquels elle s’applique ; elle ne s’applique jamais globalement.',
  },
  ja: {
    'Each advanced security rule must include at least one explicit group.':
      '高度なセキュリティルールごとに、明示的なグループを1つ以上指定してください。',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      'ルールのグループは空でない64文字以内の明示的な名前にしてください。ワイルドカードは使用できません。',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      '各ルールには適用するAPIグループを指定してください。ルールが全体に適用されることはありません。',
  },
  ru: {
    'Each advanced security rule must include at least one explicit group.':
      'Для каждого расширенного правила безопасности укажите хотя бы одну явную группу.',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      'Группы правил должны быть непустыми явными именами длиной не более 64 символов; группы с подстановочными знаками запрещены.',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      'Для каждого правила укажите группы API, к которым оно применяется; глобальное применение невозможно.',
  },
  vi: {
    'Each advanced security rule must include at least one explicit group.':
      'Mỗi quy tắc bảo mật nâng cao phải chỉ định ít nhất một nhóm cụ thể.',
    'Rule groups must be non-empty explicit names of at most 64 characters; wildcard groups are not allowed.':
      'Nhóm quy tắc phải là tên cụ thể không rỗng, dài tối đa 64 ký tự; không cho phép nhóm ký tự đại diện.',
    'Every rule must list the API group or groups it applies to; rules never apply globally.':
      'Mỗi quy tắc phải liệt kê các nhóm API mà nó áp dụng; quy tắc không bao giờ áp dụng toàn cục.',
  },
}

for (const [locale, translations] of Object.entries(discountTranslations)) {
  Object.assign(newKeys[locale], translations)
}
for (const [locale, translations] of Object.entries(regionPolicyTranslations)) {
  Object.assign(newKeys[locale], translations)
}
for (const [locale, translations] of Object.entries(
  advancedSecurityTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const aiProfileTranslations = {
  en: {
    'AI labels': 'AI labels',
    'Generated from assistant conversations':
      'Generated from assistant conversations',
    'Managed by an administrator': 'Managed by an administrator',
    'Continue with Google': 'Continue with Google',
    'Discount code': 'Discount code',
    'Discount code is invalid': 'Discount code is invalid',
    Or: 'Or',
    'Page {{page}} of {{total}}': 'Page {{page}} of {{total}}',
    'The drawing workbench is available after developer access is approved.':
      'The drawing workbench is available after developer access is approved.',
  },
  zh: {
    'AI labels': 'AI 标签',
    'Generated from assistant conversations': '根据助手对话生成',
    'Managed by an administrator': '由管理员管理',
    'Continue with Google': '使用 Google 继续',
    'Discount code': '优惠码',
    'Discount code is invalid': '优惠码无效',
    Or: '或',
    'Page {{page}} of {{total}}': '第 {{page}} / {{total}} 页',
    'The drawing workbench is available after developer access is approved.':
      '开发者访问获批后即可使用绘图工作台。',
  },
  'zh-TW': {
    'AI labels': 'AI 標籤',
    'Generated from assistant conversations': '根據助手對話產生',
    'Managed by an administrator': '由管理員管理',
    'Continue with Google': '使用 Google 繼續',
    'Discount code': '優惠碼',
    'Discount code is invalid': '優惠碼無效',
    Or: '或',
    'Page {{page}} of {{total}}': '第 {{page}} / {{total}} 頁',
    'The drawing workbench is available after developer access is approved.':
      '開發者存取獲核准後即可使用繪圖工作台。',
  },
  fr: {
    'AI labels': 'Étiquettes IA',
    'Generated from assistant conversations':
      'Générées à partir des conversations avec l’assistant',
    'Managed by an administrator': 'Gérées par un administrateur',
    'Continue with Google': 'Continuer avec Google',
    'Discount code': 'Code de réduction',
    'Discount code is invalid': 'Le code de réduction est invalide',
    Or: 'Ou',
    'Page {{page}} of {{total}}': 'Page {{page}} sur {{total}}',
    'The drawing workbench is available after developer access is approved.':
      'L’atelier de dessin est disponible après l’approbation de l’accès développeur.',
  },
  ja: {
    'AI labels': 'AIラベル',
    'Generated from assistant conversations': 'アシスタントとの会話から生成',
    'Managed by an administrator': '管理者が管理',
    'Continue with Google': 'Google で続行',
    'Discount code': '割引コード',
    'Discount code is invalid': '割引コードが無効です',
    Or: 'または',
    'Page {{page}} of {{total}}': '{{total}} ページ中 {{page}} ページ',
    'The drawing workbench is available after developer access is approved.':
      '開発者アクセスの承認後に描画ワークベンチを利用できます。',
  },
  ru: {
    'AI labels': 'Метки ИИ',
    'Generated from assistant conversations':
      'Созданы по диалогам с ассистентом',
    'Managed by an administrator': 'Управляются администратором',
    'Continue with Google': 'Продолжить через Google',
    'Discount code': 'Код скидки',
    'Discount code is invalid': 'Код скидки недействителен',
    Or: 'Или',
    'Page {{page}} of {{total}}': 'Страница {{page}} из {{total}}',
    'The drawing workbench is available after developer access is approved.':
      'Рабочая область рисования доступна после одобрения доступа разработчика.',
  },
  vi: {
    'AI labels': 'Nhãn AI',
    'Generated from assistant conversations':
      'Được tạo từ hội thoại với trợ lý',
    'Managed by an administrator': 'Do quản trị viên quản lý',
    'Continue with Google': 'Tiếp tục với Google',
    'Discount code': 'Mã giảm giá',
    'Discount code is invalid': 'Mã giảm giá không hợp lệ',
    Or: 'Hoặc',
    'Page {{page}} of {{total}}': 'Trang {{page}} / {{total}}',
    'The drawing workbench is available after developer access is approved.':
      'Có thể sử dụng bàn vẽ sau khi quyền nhà phát triển được phê duyệt.',
  },
}

for (const [locale, translations] of Object.entries(aiProfileTranslations)) {
  Object.assign(newKeys[locale], translations)
}

const channelMarketplaceTranslations = {
  en: {
    'Channel marketplace': 'Channel marketplace',
    'Share a channel': 'Share a channel',
    'All channels': 'All channels',
    'My channels': 'My channels',
    Review: 'Review',
    'Public group': 'Public group',
    'Public channel group': 'Public channel group',
    'Every submission is reviewed before it is listed.':
      'Every submission is reviewed before it is listed.',
    'The contributor account email is shown publicly.':
      'The contributor account email is shown publicly.',
    'All approved shared channels use this administrator-configured group.':
      'All approved shared channels use this administrator-configured group.',
    'Submission sent for review': 'Submission sent for review',
    'Report sent to administrators': 'Report sent to administrators',
    'Report channel': 'Report channel',
    Report: 'Report',
    'Close report': 'Close report',
    'Open reports': 'Open reports',
    'Send report': 'Send report',
    'Reason for reporting': 'Reason for reporting',
    'Review note (required when rejecting)':
      'Review note (required when rejecting)',
    'Comma-separated model IDs': 'Comma-separated model IDs',
    'No approved channels yet.': 'No approved channels yet.',
    'You have not uploaded a channel yet.':
      'You have not shared a channel yet.',
    'No description': 'No description',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.',
    '{{count}} submissions waiting for review':
      '{{count}} submissions waiting for review',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      'Tell administrators what should be checked. The report will not immediately disable the channel.',
  },
  zh: {
    'Channel marketplace': '渠道市场',
    'Share a channel': '分享渠道',
    'All channels': '所有渠道',
    'My channels': '我的渠道',
    Review: '审核',
    'Public group': '公开分组',
    'Public channel group': '公开渠道分组',
    'Every submission is reviewed before it is listed.':
      '所有提交都要审核通过后才会展示。',
    'The contributor account email is shown publicly.':
      '分享者的账号邮箱会公开显示。',
    'All approved shared channels use this administrator-configured group.':
      '所有通过审核的分享渠道都使用此管理员配置的分组。',
    'Submission sent for review': '已提交审核',
    'Report sent to administrators': '举报已发送给管理员',
    'Report channel': '举报渠道',
    Report: '举报',
    'Close report': '关闭举报',
    'Open reports': '待处理举报',
    'Send report': '发送举报',
    'Reason for reporting': '举报原因',
    'Review note (required when rejecting)': '审核备注（拒绝时必填）',
    'Comma-separated model IDs': '用逗号分隔模型 ID',
    'No approved channels yet.': '暂时没有已审核渠道。',
    'You have not uploaded a channel yet.': '你还没有分享渠道。',
    'No description': '暂无说明',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      '渠道会被分配到管理员配置的公开分组，审核通过后才会发布。请勿提交凭据。',
    '{{count}} submissions waiting for review': '{{count}} 个提交等待审核',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      '请告诉管理员需要检查什么。举报不会立即停用渠道。',
  },
  'zh-TW': {
    'Channel marketplace': '渠道市集',
    'Share a channel': '分享渠道',
    'All channels': '所有渠道',
    'My channels': '我的渠道',
    Review: '審核',
    'Public group': '公開分組',
    'Public channel group': '公開渠道分組',
    'Every submission is reviewed before it is listed.':
      '所有提交都會在審核通過後才會顯示。',
    'The contributor account email is shown publicly.':
      '分享者的帳戶電子郵件會公開顯示。',
    'All approved shared channels use this administrator-configured group.':
      '所有通過審核的分享渠道都使用管理員設定的分組。',
    'Submission sent for review': '已提交審核',
    'Report sent to administrators': '舉報已送給管理員',
    'Report channel': '舉報渠道',
    Report: '舉報',
    'Close report': '關閉舉報',
    'Open reports': '待處理舉報',
    'Send report': '送出舉報',
    'Reason for reporting': '舉報原因',
    'Review note (required when rejecting)': '審核備註（拒絕時必填）',
    'Comma-separated model IDs': '以逗號分隔模型 ID',
    'No approved channels yet.': '目前沒有已審核渠道。',
    'You have not uploaded a channel yet.': '你尚未分享渠道。',
    'No description': '暫無說明',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      '渠道會被分配至管理員設定的公開分組，審核通過後才會發布。請勿提交憑證。',
    '{{count}} submissions waiting for review': '{{count}} 個提交等待審核',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      '請告訴管理員需要檢查什麼。舉報不會立即停用渠道。',
  },
  fr: {
    'Channel marketplace': 'Marché des canaux',
    'Share a channel': 'Partager un canal',
    'All channels': 'Tous les canaux',
    'My channels': 'Mes canaux',
    Review: 'Révision',
    'Public group': 'Groupe public',
    'Public channel group': 'Groupe de canaux publics',
    'Every submission is reviewed before it is listed.':
      'Chaque soumission est vérifiée avant sa publication.',
    'The contributor account email is shown publicly.':
      'L’adresse e-mail du contributeur est affichée publiquement.',
    'All approved shared channels use this administrator-configured group.':
      'Tous les canaux partagés approuvés utilisent ce groupe configuré par l’administrateur.',
    'Submission sent for review': 'Soumission envoyée pour révision',
    'Report sent to administrators': 'Signalement envoyé aux administrateurs',
    'Report channel': 'Signaler un canal',
    Report: 'Signaler',
    'Close report': 'Fermer le signalement',
    'Open reports': 'Signalements ouverts',
    'Send report': 'Envoyer le signalement',
    'Reason for reporting': 'Motif du signalement',
    'Review note (required when rejecting)':
      'Note de révision (obligatoire en cas de refus)',
    'Comma-separated model IDs':
      'Identifiants de modèles séparés par des virgules',
    'No approved channels yet.': 'Aucun canal approuvé pour le moment.',
    'You have not uploaded a channel yet.':
      'Vous n’avez pas encore partagé de canal.',
    'No description': 'Aucune description',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      'Le canal utilise le groupe public configuré par l’administrateur et est vérifié avant publication. Ne fournissez pas d’identifiants.',
    '{{count}} submissions waiting for review':
      '{{count}} soumissions en attente de révision',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      'Indiquez aux administrateurs ce qui doit être vérifié. Le signalement ne désactivera pas immédiatement le canal.',
  },
  ja: {
    'Channel marketplace': 'チャンネルマーケット',
    'Share a channel': 'チャンネルを共有',
    'All channels': 'すべてのチャンネル',
    'My channels': '自分のチャンネル',
    Review: '審査',
    'Public group': '公開グループ',
    'Public channel group': '公開チャンネルグループ',
    'Every submission is reviewed before it is listed.':
      'すべての投稿は審査後に掲載されます。',
    'The contributor account email is shown publicly.':
      '共有者のアカウントメールアドレスは公開表示されます。',
    'All approved shared channels use this administrator-configured group.':
      '承認された共有チャンネルはすべて管理者が設定したグループを使用します。',
    'Submission sent for review': '審査に送信しました',
    'Report sent to administrators': '管理者に報告しました',
    'Report channel': 'チャンネルを報告',
    Report: '報告',
    'Close report': '報告を閉じる',
    'Open reports': '未処理の報告',
    'Send report': '報告を送信',
    'Reason for reporting': '報告理由',
    'Review note (required when rejecting)': '審査メモ（却下時は必須）',
    'Comma-separated model IDs': 'モデル ID をカンマで区切って入力',
    'No approved channels yet.': '承認済みのチャンネルはまだありません。',
    'You have not uploaded a channel yet.':
      'まだチャンネルを共有していません。',
    'No description': '説明なし',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      'チャンネルは管理者設定の公開グループに割り当てられ、掲載前に審査されます。認証情報は送信しないでください。',
    '{{count}} submissions waiting for review':
      '{{count}} 件の投稿が審査待ちです',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      '確認してほしい点を管理者に伝えてください。報告しても直ちにチャンネルは無効になりません。',
  },
  ru: {
    'Channel marketplace': 'Каталог каналов',
    'Share a channel': 'Поделиться каналом',
    'All channels': 'Все каналы',
    'My channels': 'Мои каналы',
    Review: 'Проверка',
    'Public group': 'Публичная группа',
    'Public channel group': 'Группа публичных каналов',
    'Every submission is reviewed before it is listed.':
      'Каждая заявка проверяется перед публикацией.',
    'The contributor account email is shown publicly.':
      'Электронная почта автора отображается публично.',
    'All approved shared channels use this administrator-configured group.':
      'Все одобренные каналы используют группу, настроенную администратором.',
    'Submission sent for review': 'Заявка отправлена на проверку',
    'Report sent to administrators': 'Жалоба отправлена администраторам',
    'Report channel': 'Пожаловаться на канал',
    Report: 'Пожаловаться',
    'Close report': 'Закрыть жалобу',
    'Open reports': 'Открытые жалобы',
    'Send report': 'Отправить жалобу',
    'Reason for reporting': 'Причина жалобы',
    'Review note (required when rejecting)':
      'Заметка проверки (обязательна при отклонении)',
    'Comma-separated model IDs': 'Идентификаторы моделей через запятую',
    'No approved channels yet.': 'Одобренных каналов пока нет.',
    'You have not uploaded a channel yet.': 'Вы ещё не поделились каналом.',
    'No description': 'Без описания',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      'Канал получает публичную группу, настроенную администратором, и проверяется перед публикацией. Не отправляйте учётные данные.',
    '{{count}} submissions waiting for review': 'Заявок на проверке: {{count}}',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      'Укажите администраторам, что нужно проверить. Жалоба не отключит канал немедленно.',
  },
  vi: {
    'Channel marketplace': 'Chợ kênh',
    'Share a channel': 'Chia sẻ kênh',
    'All channels': 'Tất cả kênh',
    'My channels': 'Kênh của tôi',
    Review: 'Xét duyệt',
    'Public group': 'Nhóm công khai',
    'Public channel group': 'Nhóm kênh công khai',
    'Every submission is reviewed before it is listed.':
      'Mọi đề xuất đều được xét duyệt trước khi hiển thị.',
    'The contributor account email is shown publicly.':
      'Email tài khoản của người chia sẻ sẽ được hiển thị công khai.',
    'All approved shared channels use this administrator-configured group.':
      'Mọi kênh được duyệt đều dùng nhóm do quản trị viên cấu hình.',
    'Submission sent for review': 'Đã gửi đề xuất để xét duyệt',
    'Report sent to administrators': 'Đã gửi báo cáo cho quản trị viên',
    'Report channel': 'Báo cáo kênh',
    Report: 'Báo cáo',
    'Close report': 'Đóng báo cáo',
    'Open reports': 'Báo cáo đang mở',
    'Send report': 'Gửi báo cáo',
    'Reason for reporting': 'Lý do báo cáo',
    'Review note (required when rejecting)':
      'Ghi chú xét duyệt (bắt buộc khi từ chối)',
    'Comma-separated model IDs': 'ID model, phân tách bằng dấu phẩy',
    'No approved channels yet.': 'Chưa có kênh nào được duyệt.',
    'You have not uploaded a channel yet.': 'Bạn chưa chia sẻ kênh nào.',
    'No description': 'Không có mô tả',
    'The channel is assigned to the administrator-configured public group and is reviewed before publication. Do not submit credentials.':
      'Kênh được gán vào nhóm công khai do quản trị viên cấu hình và được xét duyệt trước khi đăng. Không gửi thông tin xác thực.',
    '{{count}} submissions waiting for review':
      '{{count}} đề xuất đang chờ xét duyệt',
    'Tell administrators what should be checked. The report will not immediately disable the channel.':
      'Cho quản trị viên biết cần kiểm tra điều gì. Báo cáo sẽ không vô hiệu hóa kênh ngay lập tức.',
  },
}

const assistantAndSecurityTranslations = {
  en: {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      'Sensitive details are hidden until confirmation and remain visible only to you.',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      'This history is available because the account has a lower access level. Credential details remain visible only to their owner.',
    'The credential is shown only after confirmation and is never added to chat history.':
      'The credential is shown only after confirmation and is never added to chat history.',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      'Click Import to CC Switch, then review and confirm the import dialog.',
    'Please enter a message.': 'Please enter a message.',
    'Please enter a message other than a single punctuation mark.':
      'Please enter a message other than a single punctuation mark.',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.',
    'Enter the code from your authenticator app or a backup code.':
      'Enter the code from your authenticator app or a backup code.',
    'Enter verification code or backup code':
      'Enter verification code or backup code',
  },
  zh: {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      '敏感信息已隐藏，确认后仅向你显示，并且只对你可见。',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      '请勿在聊天中发送个人信息、密码、API 密钥或凭证。本站凭证仅在你明确确认后向你显示，只对你可见，并且不会进入助手上下文。',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      '由于该账号等级较低，你可以查看此历史；凭证详情仍仅对其所有者可见。',
    'The credential is shown only after confirmation and is never added to chat history.':
      '凭证仅在确认后显示，绝不会加入聊天记录。',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      '请先创建 API 密钥，然后确认导入 CC Switch。浏览器会根据所选模型和服务根地址生成链接，密钥不会进入助手对话。',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      '点击“导入 CC Switch”，然后检查并确认导入对话框。',
    'Please enter a message.': '请输入消息。',
    'Please enter a message other than a single punctuation mark.':
      '请输入不只是单个标点符号的消息。',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      '该申请已在管理员队列中。你可以继续对话，之后再补充 AI 推荐信；推荐信是可选的。',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      '你可以不附带 AI 推荐信，直接提交管理员审核。推荐信只为审核者提供更多背景，不会决定是否通过。',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      '请在个人资料中绑定邮箱、启用双重身份验证或设置 Passkey，以解锁敏感操作。',
    'Enter the code from your authenticator app or a backup code.':
      '请输入身份验证器应用中的代码或备用代码。',
    'Enter verification code or backup code': '请输入验证码或备用代码',
  },
  'zh-TW': {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      '敏感資訊已隱藏，確認後僅向你顯示，且只有你可見。',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      '請勿在聊天中傳送個人資料、密碼、API 金鑰或憑證。本站憑證僅在你明確確認後向你顯示，只有你可見，且不會進入助理上下文。',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      '因該帳號等級較低，你可以查看此歷史；憑證詳情仍僅對其擁有者可見。',
    'The credential is shown only after confirmation and is never added to chat history.':
      '憑證僅在確認後顯示，絕不會加入聊天記錄。',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      '請先建立 API 金鑰，然後確認匯入 CC Switch。瀏覽器會根據所選模型和服務根網址產生連結，金鑰不會進入助理對話。',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      '點擊「匯入 CC Switch」，然後檢查並確認匯入對話框。',
    'Please enter a message.': '請輸入訊息。',
    'Please enter a message other than a single punctuation mark.':
      '請輸入不只是單一標點符號的訊息。',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      '此申請已在管理員佇列中。你可以繼續對話，之後再補充 AI 推薦信；推薦信為選填。',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      '你可以不附帶 AI 推薦信，直接提交管理員審核。推薦信只提供更多背景，不會決定是否核准。',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      '請在個人資料中綁定電子郵件、啟用雙重驗證或設定 Passkey，以解鎖敏感操作。',
    'Enter the code from your authenticator app or a backup code.':
      '請輸入驗證器應用程式中的代碼或備用代碼。',
    'Enter verification code or backup code': '請輸入驗證碼或備用代碼',
  },
  fr: {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      'Les informations sensibles sont masquées jusqu’à confirmation et restent visibles uniquement pour vous.',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      'N’envoyez pas d’informations personnelles, de mots de passe, de clés API ou d’identifiants dans le chat. Les identifiants fournis par le site ne sont affichés qu’après votre confirmation explicite, restent visibles uniquement pour vous et restent hors du contexte de l’assistant.',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      'Cet historique est disponible car le compte a un niveau d’accès inférieur. Les détails d’identification restent visibles uniquement par leur propriétaire.',
    'The credential is shown only after confirmation and is never added to chat history.':
      'L’identifiant n’est affiché qu’après confirmation et n’est jamais ajouté à l’historique du chat.',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      'Créez d’abord une clé API, puis confirmez l’importation dans CC Switch. Le navigateur construit le lien avec le modèle et la racine du service sélectionnés ; la clé n’entre jamais dans le chat de l’assistant.',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      'Cliquez sur « Importer dans CC Switch », puis vérifiez et confirmez la boîte de dialogue d’importation.',
    'Please enter a message.': 'Saisissez un message.',
    'Please enter a message other than a single punctuation mark.':
      'Saisissez un message autre qu’un simple signe de ponctuation.',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      'La demande est déjà dans la file d’attente de l’administrateur. Une recommandation IA est facultative et peut être ajoutée après la poursuite de la conversation.',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      'Vous pouvez soumettre la demande à l’administrateur sans recommandation IA. Celle-ci apporte du contexte au réviseur, mais ne décide jamais de l’accès.',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      'Associez un e-mail, activez la 2FA ou configurez une Passkey dans votre profil pour débloquer les opérations sensibles.',
    'Enter the code from your authenticator app or a backup code.':
      'Saisissez le code de votre application d’authentification ou un code de secours.',
    'Enter verification code or backup code':
      'Saisissez le code de vérification ou de secours',
  },
  ja: {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      '機密情報は確認するまで非表示で、確認後もあなたにだけ表示されます。',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      'チャットに個人情報、パスワード、API キー、認証情報を送信しないでください。サイトが発行する認証情報は明示的な確認後にのみ表示され、あなたにだけ表示され、アシスタントのコンテキストには入りません。',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      'この履歴はアカウントのアクセスレベルが低いため表示されています。認証情報の詳細は所有者だけに表示されます。',
    'The credential is shown only after confirmation and is never added to chat history.':
      '認証情報は確認後にのみ表示され、チャット履歴には追加されません。',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      'まず API キーを作成し、CC Switch へのインポートを確認してください。ブラウザーが選択したモデルとサービスルートからリンクを作成し、キーがアシスタントのチャットに入ることはありません。',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      '「CC Switch にインポート」をクリックし、インポート確認ダイアログを確認して承認してください。',
    'Please enter a message.': 'メッセージを入力してください。',
    'Please enter a message other than a single punctuation mark.':
      '句読点1文字だけではないメッセージを入力してください。',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      '申請はすでに管理者キューに入っています。AI 推薦文は任意で、会話を続けた後に追加できます。',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      'AI 推薦文なしで管理者審査に提出できます。推薦文は審査の参考情報であり、アクセス可否を決めるものではありません。',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      'プロフィールでメールアドレスを連携し、2FA を有効にするか Passkey を設定すると、機密操作を利用できます。',
    'Enter the code from your authenticator app or a backup code.':
      '認証アプリのコードまたはバックアップコードを入力してください。',
    'Enter verification code or backup code':
      '確認コードまたはバックアップコードを入力してください',
  },
  ru: {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      'Чувствительные данные скрыты до подтверждения и после него видны только вам.',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      'Не отправляйте в чат личные данные, пароли, API-ключи или учётные данные. Выданные сайтом учётные данные показываются только после явного подтверждения, видны только вам и не попадают в контекст помощника.',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      'Эта история доступна из-за более низкого уровня доступа аккаунта. Сведения об учётных данных видны только их владельцу.',
    'The credential is shown only after confirmation and is never added to chat history.':
      'Учётные данные показываются только после подтверждения и никогда не добавляются в историю чата.',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      'Сначала создайте API-ключ, затем подтвердите импорт в CC Switch. Браузер создаёт ссылку на основе выбранной модели и корня сервиса; ключ никогда не попадает в чат помощника.',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      'Нажмите «Импорт в CC Switch», затем проверьте и подтвердите диалог импорта.',
    'Please enter a message.': 'Введите сообщение.',
    'Please enter a message other than a single punctuation mark.':
      'Введите сообщение, состоящее не только из одного знака препинания.',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      'Запрос уже находится в очереди администратора. Рекомендация ИИ необязательна и может быть добавлена после продолжения диалога.',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      'Можно отправить запрос администратору без рекомендации ИИ. Рекомендация лишь даёт проверяющему дополнительный контекст и не определяет доступ.',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      'Привяжите электронную почту, включите 2FA или настройте Passkey в профиле, чтобы разблокировать чувствительные операции.',
    'Enter the code from your authenticator app or a backup code.':
      'Введите код из приложения-аутентификатора или резервный код.',
    'Enter verification code or backup code':
      'Введите код подтверждения или резервный код',
  },
  vi: {
    'Sensitive details are hidden until confirmation and remain visible only to you.':
      'Thông tin nhạy cảm được ẩn cho đến khi xác nhận và chỉ hiển thị với bạn.',
    'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials are shown only after your explicit confirmation, remain visible only to you, and stay out of the assistant context.':
      'Không gửi thông tin cá nhân, mật khẩu, API key hoặc thông tin xác thực trong cuộc trò chuyện. Thông tin xác thực do trang cấp chỉ hiển thị sau khi bạn xác nhận rõ ràng, chỉ bạn có thể xem và không đi vào ngữ cảnh của trợ lý.',
    'This history is available because the account has a lower access level. Credential details remain visible only to their owner.':
      'Lịch sử này khả dụng vì tài khoản có cấp truy cập thấp hơn. Chi tiết thông tin xác thực chỉ hiển thị với chủ sở hữu.',
    'The credential is shown only after confirmation and is never added to chat history.':
      'Thông tin xác thực chỉ hiển thị sau khi xác nhận và không bao giờ được thêm vào lịch sử trò chuyện.',
    'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.':
      'Trước tiên hãy tạo API key, sau đó xác nhận việc nhập vào CC Switch. Trình duyệt tạo liên kết từ model và URL gốc của dịch vụ đã chọn; key không bao giờ đi vào cuộc trò chuyện với trợ lý.',
    'Click Import to CC Switch, then review and confirm the import dialog.':
      'Nhấp vào Nhập vào CC Switch, sau đó kiểm tra và xác nhận hộp thoại nhập.',
    'Please enter a message.': 'Vui lòng nhập tin nhắn.',
    'Please enter a message other than a single punctuation mark.':
      'Vui lòng nhập tin nhắn không chỉ gồm một dấu câu.',
    'The request is already in the administrator queue. An AI recommendation is optional and may be added after you continue the conversation.':
      'Yêu cầu đã nằm trong hàng đợi quản trị viên. Đề xuất AI là tùy chọn và có thể được thêm sau khi bạn tiếp tục cuộc trò chuyện.',
    'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.':
      'Bạn có thể gửi yêu cầu để quản trị viên xét duyệt mà không cần đề xuất AI. Đề xuất chỉ cung cấp thêm ngữ cảnh và không quyết định quyền truy cập.',
    'Bind an email, enable 2FA, or set up a Passkey in your profile to unlock sensitive operations.':
      'Hãy liên kết email, bật 2FA hoặc thiết lập Passkey trong hồ sơ để mở khóa các thao tác nhạy cảm.',
    'Enter the code from your authenticator app or a backup code.':
      'Nhập mã từ ứng dụng xác thực hoặc mã dự phòng.',
    'Enter verification code or backup code':
      'Nhập mã xác minh hoặc mã dự phòng',
  },
}

for (const [locale, translations] of Object.entries(
  assistantAndSecurityTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

for (const [locale, translations] of Object.entries(
  channelMarketplaceTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

// Routing/review strings are kept in one small shared vocabulary so every
// locale gets a complete key set even when a new marketplace control ships.
// The Chinese copy is overridden below; other locales intentionally fall back
// to the stable English label until a native translation is supplied.
const channelRoutingFallback = {
  'Channel routing': 'Channel routing',
  'Configure your own public pool. It affects only your requests; administrator routing priority remains unchanged.':
    'Configure your own public pool. It affects only your requests; administrator routing priority remains unchanged.',
  'Save routing': 'Save routing',
  Disabled: 'Disabled',
  Enabled: 'Enabled',
  'Move up': 'Move up',
  'Move down': 'Move down',
  'No linked public channels yet.': 'No linked public channels yet.',
  'Top rated': 'Top rated',
  'Recently updated': 'Recently updated',
  'Most models': 'Most models',
  'No models listed': 'No models listed',
  'Review channel': 'Review channel',
  Rating: 'Rating',
  'Write a comment (optional)': 'Write a comment (optional)',
  'Recent comments': 'Recent comments',
  'No reviews yet.': 'No reviews yet.',
  'Submit review': 'Submit review',
  'Review submitted': 'Review submitted',
  'Routing preferences saved': 'Routing preferences saved',
  'Tip contributor': 'Tip contributor',
  'Use your balance to thank this contributor. Tips are transferred immediately and cannot be reversed.':
    'Use your balance to thank this contributor. Tips are transferred immediately and cannot be reversed.',
  'Tip amount': 'Tip amount',
  'Custom tip amount': 'Custom tip amount',
  'Message (optional)': 'Message (optional)',
  'Leave a short thank-you message': 'Leave a short thank-you message',
  'Send tip': 'Send tip',
  'Tip sent': 'Tip sent',
  Tips: 'Tips',
  'Withdraw tips': 'Withdraw tips',
  'Tips withdrawn': 'Tips withdrawn',
  'Move available tips into your balance. Choose the group you want to use for future requests.':
    'Move available tips into your balance. Choose the group you want to use for future requests.',
  'Target group': 'Target group',
  'Select a group': 'Select a group',
  Withdraw: 'Withdraw',
  'Group warning': 'Group warning',
  'Confirmation {{current}} of {{total}}':
    'Confirmation {{current}} of {{total}}',
  'I understand, continue': 'I understand, continue',
}
for (const locale of Object.keys(newKeys)) {
  Object.assign(newKeys[locale], channelRoutingFallback)
}
Object.assign(newKeys.zh, {
  'Channel routing': '渠道路由',
  'Configure your own public pool. It affects only your requests; administrator routing priority remains unchanged.':
    '配置你自己的公开渠道池，只影响你的请求，不会改变管理员设置的全局路由优先级。',
  'Save routing': '保存路由',
  Disabled: '已禁用',
  Enabled: '已启用',
  'Move up': '上移',
  'Move down': '下移',
  'No linked public channels yet.': '还没有已关联的公开渠道。',
  'Top rated': '评分最高',
  'Recently updated': '最近更新',
  'Most models': '模型最多',
  'No models listed': '未列出模型',
  'Review channel': '评价渠道',
  Rating: '评分',
  'Write a comment (optional)': '写下评论（可选）',
  'Recent comments': '最近评论',
  'No reviews yet.': '暂无评论。',
  'Submit review': '提交评价',
  'Review submitted': '评价已提交',
  'Routing preferences saved': '路由偏好已保存',
  'Tip contributor': '打赏分享者',
  'Use your balance to thank this contributor. Tips are transferred immediately and cannot be reversed.':
    '使用你的余额感谢分享者。打赏会立即转账，且无法撤回。',
  'Tip amount': '打赏金额',
  'Custom tip amount': '自定义金额',
  'Message (optional)': '留言（可选）',
  'Leave a short thank-you message': '写一句感谢的话',
  'Send tip': '发送打赏',
  'Tip sent': '打赏已发送',
  Tips: '打赏收入',
  'Withdraw tips': '提取打赏',
  'Tips withdrawn': '打赏已提取',
  'Move available tips into your balance. Choose the group you want to use for future requests.':
    '将可用打赏转入你的余额，并选择之后使用的分组。',
  'Target group': '目标分组',
  'Select a group': '选择分组',
  Withdraw: '提取',
  'Group warning': '分组警告',
  'Confirmation {{current}} of {{total}}': '第 {{current}}/{{total}} 次确认',
  'I understand, continue': '我已了解，继续',
})

const platformSkillTranslations = {
  en: {
    'Platform skill files': 'Platform skill files',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.',
    'Add file': 'Add file',
    'No platform skill files yet.': 'No platform skill files yet.',
    Off: 'Off',
    'Skill file path': 'Skill file path',
    'Delete skill file': 'Delete skill file',
    'Skill file content': 'Skill file content',
    'Use this platform skill': 'Use this platform skill',
    'Maximum 32 files / 32000 characters total':
      'Maximum 32 files / 32000 characters total',
    'Add a file to edit a platform skill.':
      'Add a file to edit a platform skill.',
  },
  zh: {
    'Platform skill files': '平台技能文件',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      '这些是平台助手共享的受限虚拟文件，不会授予文件系统或工具权限。',
    'Add file': '添加文件',
    'No platform skill files yet.': '暂时没有平台技能文件。',
    Off: '停用',
    'Skill file path': '技能文件路径',
    'Delete skill file': '删除技能文件',
    'Skill file content': '技能文件内容',
    'Use this platform skill': '启用此平台技能',
    'Maximum 32 files / 32000 characters total':
      '最多 32 个文件 / 总计 32000 个字符',
    'Add a file to edit a platform skill.': '添加文件后即可编辑平台技能。',
  },
  'zh-TW': {
    'Platform skill files': '平台技能檔案',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      '這些是平台助手共用的受限虛擬檔案，不會授予檔案系統或工具權限。',
    'Add file': '新增檔案',
    'No platform skill files yet.': '目前沒有平台技能檔案。',
    Off: '停用',
    'Skill file path': '技能檔案路徑',
    'Delete skill file': '刪除技能檔案',
    'Skill file content': '技能檔案內容',
    'Use this platform skill': '啟用此平台技能',
    'Maximum 32 files / 32000 characters total':
      '最多 32 個檔案 / 共 32000 個字元',
    'Add a file to edit a platform skill.': '新增檔案後即可編輯平台技能。',
  },
  fr: {
    'Platform skill files': 'Fichiers de compétences de la plateforme',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      "Ces fichiers virtuels limités sont partagés par l'assistant de la plateforme. Ils n'accordent aucun accès aux fichiers ni aux outils.",
    'Add file': 'Ajouter un fichier',
    'No platform skill files yet.':
      "Aucun fichier de compétence pour l'instant.",
    Off: 'Désactivé',
    'Skill file path': 'Chemin du fichier de compétence',
    'Delete skill file': 'Supprimer le fichier de compétence',
    'Skill file content': 'Contenu du fichier de compétence',
    'Use this platform skill': 'Utiliser cette compétence',
    'Maximum 32 files / 32000 characters total':
      'Maximum 32 fichiers / 32000 caractères au total',
    'Add a file to edit a platform skill.':
      'Ajoutez un fichier pour modifier une compétence.',
  },
  ja: {
    'Platform skill files': 'プラットフォームスキルファイル',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      'プラットフォーム助手が共有する上限付きの仮想ファイルです。ファイルシステムやツールの権限は付与しません。',
    'Add file': 'ファイルを追加',
    'No platform skill files yet.':
      'プラットフォームスキルファイルはまだありません。',
    Off: '無効',
    'Skill file path': 'スキルファイルのパス',
    'Delete skill file': 'スキルファイルを削除',
    'Skill file content': 'スキルファイルの内容',
    'Use this platform skill': 'このプラットフォームスキルを使用',
    'Maximum 32 files / 32000 characters total':
      '最大 32 ファイル / 合計 32000 文字',
    'Add a file to edit a platform skill.':
      'ファイルを追加するとスキルを編集できます。',
  },
  ru: {
    'Platform skill files': 'Файлы навыков платформы',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      'Это ограниченные виртуальные файлы общего помощника платформы. Они не дают доступа к файлам или инструментам.',
    'Add file': 'Добавить файл',
    'No platform skill files yet.': 'Файлов навыков платформы пока нет.',
    Off: 'Выкл.',
    'Skill file path': 'Путь к файлу навыка',
    'Delete skill file': 'Удалить файл навыка',
    'Skill file content': 'Содержимое файла навыка',
    'Use this platform skill': 'Использовать этот навык',
    'Maximum 32 files / 32000 characters total':
      'Не более 32 файлов / 32000 символов всего',
    'Add a file to edit a platform skill.':
      'Добавьте файл, чтобы изменить навык платформы.',
  },
  vi: {
    'Platform skill files': 'Tệp kỹ năng nền tảng',
    'These are bounded virtual files shared by the platform assistant. They never grant filesystem or tool permissions.':
      'Đây là các tệp ảo có giới hạn được trợ lý nền tảng dùng chung. Chúng không cấp quyền tệp hoặc công cụ.',
    'Add file': 'Thêm tệp',
    'No platform skill files yet.': 'Chưa có tệp kỹ năng nền tảng.',
    Off: 'Tắt',
    'Skill file path': 'Đường dẫn tệp kỹ năng',
    'Delete skill file': 'Xóa tệp kỹ năng',
    'Skill file content': 'Nội dung tệp kỹ năng',
    'Use this platform skill': 'Dùng kỹ năng nền tảng này',
    'Maximum 32 files / 32000 characters total':
      'Tối đa 32 tệp / tổng cộng 32000 ký tự',
    'Add a file to edit a platform skill.':
      'Thêm tệp để chỉnh sửa kỹ năng nền tảng.',
  },
}
for (const [locale, translations] of Object.entries(
  platformSkillTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

for (const [locale, translations] of Object.entries(todoTranslations)) {
  Object.assign(newKeys[locale], translations)
}

const drawingLayoutTranslations = {
  en: {
    'Be specific about the subject, mood, and style.':
      'Be specific about the subject, mood, and style.',
    'Generated images': 'Generated images',
    'Describe an image, choose a group, and generate a preview.':
      'Describe an image, choose a group, and generate a preview.',
    'Search by username, name, or email': 'Search by username, name, or email',
  },
  zh: {
    'Be specific about the subject, mood, and style.':
      '可以补充主体、氛围和风格，让结果更贴近你的想法。',
    'Generated images': '生成结果',
    'Describe an image, choose a group, and generate a preview.':
      '描述图片，选择分组，然后生成预览。',
    'Search by username, name, or email': '按用户名、姓名或邮箱搜索',
  },
  'zh-TW': {
    'Be specific about the subject, mood, and style.':
      '可以補充主體、氛圍和風格，讓結果更貼近你的想法。',
    'Generated images': '生成結果',
    'Describe an image, choose a group, and generate a preview.':
      '描述圖片、選擇分組，然後生成預覽。',
    'Search by username, name, or email': '按使用者名稱、姓名或電子郵件搜尋',
  },
  fr: {
    'Be specific about the subject, mood, and style.':
      'Précisez le sujet, l’ambiance et le style.',
    'Generated images': 'Images générées',
    'Describe an image, choose a group, and generate a preview.':
      'Décrivez une image, choisissez un groupe, puis générez un aperçu.',
    'Search by username, name, or email':
      "Rechercher par nom d'utilisateur, nom ou e-mail",
  },
  ja: {
    'Be specific about the subject, mood, and style.':
      '被写体、雰囲気、スタイルを具体的に指定してください。',
    'Generated images': '生成結果',
    'Describe an image, choose a group, and generate a preview.':
      '画像を説明し、グループを選んでプレビューを生成します。',
    'Search by username, name, or email':
      'ユーザー名、氏名、メールアドレスで検索',
  },
  ru: {
    'Be specific about the subject, mood, and style.':
      'Уточните объект, настроение и стиль.',
    'Generated images': 'Созданные изображения',
    'Describe an image, choose a group, and generate a preview.':
      'Опишите изображение, выберите группу и создайте предварительный просмотр.',
    'Search by username, name, or email':
      'Поиск по имени пользователя, имени или электронной почте',
  },
  vi: {
    'Be specific about the subject, mood, and style.':
      'Hãy nêu rõ chủ thể, không khí và phong cách.',
    'Generated images': 'Ảnh đã tạo',
    'Describe an image, choose a group, and generate a preview.':
      'Mô tả hình ảnh, chọn nhóm rồi tạo bản xem trước.',
    'Search by username, name, or email':
      'Tìm theo tên người dùng, tên hoặc email',
  },
}
for (const [locale, translations] of Object.entries(
  drawingLayoutTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const registrationChannelTranslations = {
  en: {
    'Registration channels': 'Registration channels',
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.',
    'Allow new accounts through {{method}}':
      'Allow new accounts through {{method}}',
  },
  zh: {
    'Registration channels': '注册渠道',
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      '仅对新注册停用所选 OAuth 渠道，现有用户仍可使用这些渠道登录。',
    'Allow new accounts through {{method}}': '允许通过 {{method}} 创建新账号',
  },
  'zh-TW': {
    'Registration channels': '註冊管道',
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      '僅對新註冊停用所選 OAuth 管道，現有使用者仍可使用這些管道登入。',
    'Allow new accounts through {{method}}': '允許透過 {{method}} 建立新帳號',
  },
  fr: {
    'Registration channels': "Canaux d'inscription",
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      'Désactiver les canaux OAuth sélectionnés uniquement pour les nouvelles inscriptions. Les comptes existants peuvent toujours s’y connecter.',
    'Allow new accounts through {{method}}':
      'Autoriser la création de comptes via {{method}}',
  },
  ja: {
    'Registration channels': '登録チャネル',
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      '選択した OAuth チャネルを新規登録にのみ無効化します。既存ユーザーは引き続きこれらのチャネルでログインできます。',
    'Allow new accounts through {{method}}':
      '{{method}} で新しいアカウントを作成可能',
  },
  ru: {
    'Registration channels': 'Каналы регистрации',
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      'Отключает выбранные OAuth-каналы только для новых регистраций. Существующие пользователи по-прежнему могут входить через них.',
    'Allow new accounts through {{method}}':
      'Разрешить создание новых аккаунтов через {{method}}',
  },
  vi: {
    'Registration channels': 'Kênh đăng ký',
    'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.':
      'Tắt các kênh OAuth đã chọn chỉ đối với đăng ký mới. Người dùng hiện tại vẫn có thể đăng nhập bằng các kênh này.',
    'Allow new accounts through {{method}}':
      'Cho phép tạo tài khoản mới qua {{method}}',
  },
}
for (const [locale, translations] of Object.entries(
  registrationChannelTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const reasoningEffortTranslations = {
  en: {
    'Auto (model default)': 'Auto (model default)',
    'None (no reasoning)': 'None (no reasoning)',
    Low: 'Low',
    Medium: 'Medium',
    High: 'High',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.',
  },
  zh: {
    'Auto (model default)': '自动（使用模型默认值）',
    'None (no reasoning)': '无（不启用思考）',
    Low: '低',
    Medium: '中',
    High: '高',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      '控制助手请求的默认思考提示。自动模式会使用每个模型的原生默认值。',
  },
  'zh-TW': {
    'Auto (model default)': '自動（使用模型預設值）',
    'None (no reasoning)': '無（不啟用推理）',
    Low: '低',
    Medium: '中',
    High: '高',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      '控制助手請求的預設推理提示。自動模式會使用每個模型的原生預設值。',
  },
  fr: {
    'Auto (model default)': 'Automatique (valeur du modèle)',
    'None (no reasoning)': 'Aucun (sans raisonnement)',
    Low: 'Faible',
    Medium: 'Moyen',
    High: 'Élevé',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      'Contrôle l’indication de raisonnement par défaut des requêtes. Le mode automatique utilise la valeur native de chaque modèle.',
  },
  ja: {
    'Auto (model default)': '自動（モデルの既定値）',
    'None (no reasoning)': 'なし（推論しない）',
    Low: '低',
    Medium: '中',
    High: '高',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      'アシスタント要求に送る既定の推論ヒントを制御します。自動では各モデルの既定値を使用します。',
  },
  ru: {
    'Auto (model default)': 'Авто (настройка модели)',
    'None (no reasoning)': 'Нет (без рассуждений)',
    Low: 'Низкая',
    Medium: 'Средняя',
    High: 'Высокая',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      'Задаёт подсказку для рассуждений в запросах помощника. Режим «Авто» использует значение модели.',
  },
  vi: {
    'Auto (model default)': 'Tự động (mặc định của mô hình)',
    'None (no reasoning)': 'Không (không suy luận)',
    Low: 'Thấp',
    Medium: 'Trung bình',
    High: 'Cao',
    'Controls the default reasoning hint sent with assistant requests. Auto lets each model use its native default.':
      'Điều khiển gợi ý suy luận mặc định trong yêu cầu trợ lý. Tự động sẽ dùng mặc định gốc của từng mô hình.',
  },
}
for (const [locale, translations] of Object.entries(
  reasoningEffortTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const requestReviewTranslations = {
  en: {
    Violations: 'Violations',
    'Assistant review logs': 'Assistant review logs',
    'Current violations': 'Current violations',
    'Unable to load review logs': 'Unable to load review logs',
    'Unable to reset violations': 'Unable to reset violations',
    'Violation count reset': 'Violation count reset',
    'No sampled reviews': 'No sampled reviews',
    Violation: 'Violation',
    'No violation': 'No violation',
    'Possible abuse': 'Possible abuse',
    'Default group': 'Default group',
    Rules: 'Rules',
    Explanation: 'Explanation',
    'Request preview': 'Request preview',
    'Resetting...': 'Resetting...',
    'Reset count': 'Reset count',
    'Per-request review probability (%)': 'Per-request review probability (%)',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.',
    'Review model': 'Review model',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      'Use an exact billable model ID. The default is deepseek-v4-flash.',
    'Per-group review policies': 'Per-group review policies',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.',
  },
  zh: {
    Violations: '违规次数',
    'Assistant review logs': '助手审查日志',
    'Current violations': '当前违规次数',
    'Unable to load review logs': '无法加载审查日志',
    'Unable to reset violations': '无法重置违规次数',
    'Violation count reset': '违规次数已重置',
    'No sampled reviews': '暂无抽样审查记录',
    Violation: '违规',
    'No violation': '未发现违规',
    'Possible abuse': '可能滥用',
    'Default group': '默认分组',
    Rules: '规则',
    Explanation: '说明',
    'Request preview': '请求摘要',
    'Resetting...': '重置中……',
    'Reset count': '重置计数',
    'Per-request review probability (%)': '每请求审查概率（%）',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 表示关闭抽样审查；1.0 约等于 1%。审查在后台运行，不会延迟响应。',
    'Review model': '审查模型',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      '填写准确且已计费的模型 ID，默认使用 deepseek-v4-flash。',
    'Per-group review policies': '分组审查策略',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      '可选 JSON，键为路由分组。每项支持 0–100 的概率及 off、low、standard、high 强度；未列出的分组使用全局概率。',
  },
  'zh-TW': {
    Violations: '違規次數',
    'Assistant review logs': '助手審查日誌',
    'Current violations': '目前違規次數',
    'Unable to load review logs': '無法載入審查日誌',
    'Unable to reset violations': '無法重置違規次數',
    'Violation count reset': '違規次數已重置',
    'No sampled reviews': '尚無抽樣審查記錄',
    Violation: '違規',
    'No violation': '未發現違規',
    'Possible abuse': '可能濫用',
    'Default group': '預設分組',
    Rules: '規則',
    Explanation: '說明',
    'Request preview': '請求摘要',
    'Resetting...': '重置中……',
    'Reset count': '重置計數',
    'Per-request review probability (%)': '每次請求審查機率（%）',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 表示停用抽樣審查；1.0 約等於 1%。審查在背景執行，不會延遲回應。',
    'Review model': '審查模型',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      '請填寫準確且可計費的模型 ID，預設使用 deepseek-v4-flash。',
    'Per-group review policies': '分組審查策略',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      '可選 JSON，鍵為路由分組。每項支援 0–100 的機率及 off、low、standard、high 強度；未列出的分組使用全域機率。',
  },
  fr: {
    Violations: 'Infractions',
    'Assistant review logs': 'Journaux de contrôle de l’assistant',
    'Current violations': 'Infractions actuelles',
    'Unable to load review logs': 'Impossible de charger les journaux',
    'Unable to reset violations': 'Impossible de réinitialiser les infractions',
    'Violation count reset': 'Compteur d’infractions réinitialisé',
    'No sampled reviews': 'Aucun contrôle échantillonné',
    Violation: 'Infraction',
    'No violation': 'Aucune infraction',
    'Possible abuse': 'Abus possible',
    'Default group': 'Groupe par défaut',
    Rules: 'Règles',
    Explanation: 'Explication',
    'Request preview': 'Aperçu de la requête',
    'Resetting...': 'Réinitialisation…',
    'Reset count': 'Réinitialiser le compteur',
    'Per-request review probability (%)':
      'Probabilité de contrôle par requête (%)',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 désactive les contrôles échantillonnés. 1,0 correspond à environ 1 % ; ils s’exécutent en arrière-plan sans retarder la réponse.',
    'Review model': 'Modèle de contrôle',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      'Utilisez un identifiant de modèle facturable exact. La valeur par défaut est deepseek-v4-flash.',
    'Per-group review policies': 'Politiques de contrôle par groupe',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      'JSON facultatif indexé par groupe de routage. Chaque valeur accepte une probabilité de 0 à 100 et une intensité off, low, standard ou high. Les groupes absents utilisent la probabilité globale.',
  },
  ja: {
    Violations: '違反回数',
    'Assistant review logs': 'アシスタント審査ログ',
    'Current violations': '現在の違反回数',
    'Unable to load review logs': '審査ログを読み込めません',
    'Unable to reset violations': '違反回数をリセットできません',
    'Violation count reset': '違反回数をリセットしました',
    'No sampled reviews': '抽出審査の記録はありません',
    Violation: '違反',
    'No violation': '違反なし',
    'Possible abuse': '不正利用の可能性',
    'Default group': '既定のグループ',
    Rules: 'ルール',
    Explanation: '説明',
    'Request preview': 'リクエスト概要',
    'Resetting...': 'リセット中…',
    'Reset count': '回数をリセット',
    'Per-request review probability (%)': 'リクエストごとの審査確率（%）',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 で抽出審査を無効にします。1.0 は約 1% です。審査はバックグラウンドで実行され、応答を遅延させません。',
    'Review model': '審査モデル',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      '課金対象の正確なモデル ID を指定します。既定値は deepseek-v4-flash です。',
    'Per-group review policies': 'グループ別審査ポリシー',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      'ルーティンググループをキーにした任意の JSON です。確率 0～100 と強度 off、low、standard、high を指定できます。未指定のグループは全体の確率を使います。',
  },
  ru: {
    Violations: 'Нарушения',
    'Assistant review logs': 'Журналы проверки помощника',
    'Current violations': 'Текущие нарушения',
    'Unable to load review logs': 'Не удалось загрузить журналы',
    'Unable to reset violations': 'Не удалось сбросить нарушения',
    'Violation count reset': 'Счётчик нарушений сброшен',
    'No sampled reviews': 'Выборочных проверок нет',
    Violation: 'Нарушение',
    'No violation': 'Нарушений нет',
    'Possible abuse': 'Возможное злоупотребление',
    'Default group': 'Группа по умолчанию',
    Rules: 'Правила',
    Explanation: 'Пояснение',
    'Request preview': 'Предпросмотр запроса',
    'Resetting...': 'Сброс…',
    'Reset count': 'Сбросить счётчик',
    'Per-request review probability (%)': 'Вероятность проверки запроса (%)',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 отключает выборочные проверки. 1,0 означает примерно 1%; проверки выполняются в фоне и не задерживают ответ.',
    'Review model': 'Модель проверки',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      'Укажите точный идентификатор оплачиваемой модели. По умолчанию используется deepseek-v4-flash.',
    'Per-group review policies': 'Политики проверки по группам',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      'Необязательный JSON с ключами групп маршрутизации. Для каждой записи задаются вероятность 0–100 и интенсивность off, low, standard или high. Для остальных групп используется общая вероятность.',
  },
  vi: {
    Violations: 'Số lần vi phạm',
    'Assistant review logs': 'Nhật ký kiểm duyệt trợ lý',
    'Current violations': 'Số vi phạm hiện tại',
    'Unable to load review logs': 'Không thể tải nhật ký kiểm duyệt',
    'Unable to reset violations': 'Không thể đặt lại số lần vi phạm',
    'Violation count reset': 'Đã đặt lại số lần vi phạm',
    'No sampled reviews': 'Chưa có kiểm duyệt lấy mẫu',
    Violation: 'Vi phạm',
    'No violation': 'Không vi phạm',
    'Possible abuse': 'Có thể lạm dụng',
    'Default group': 'Nhóm mặc định',
    Rules: 'Quy tắc',
    Explanation: 'Giải thích',
    'Request preview': 'Tóm tắt yêu cầu',
    'Resetting...': 'Đang đặt lại…',
    'Reset count': 'Đặt lại số lần',
    'Per-request review probability (%)': 'Xác suất kiểm duyệt mỗi yêu cầu (%)',
    '0 disables sampled reviews. 1.0 means roughly one percent; reviews run in the background and never delay the response.':
      '0 tắt kiểm duyệt lấy mẫu. 1.0 tương đương khoảng 1%; kiểm duyệt chạy nền và không làm chậm phản hồi.',
    'Review model': 'Model kiểm duyệt',
    'Use an exact billable model ID. The default is deepseek-v4-flash.':
      'Dùng đúng ID model có tính phí. Mặc định là deepseek-v4-flash.',
    'Per-group review policies': 'Chính sách kiểm duyệt theo nhóm',
    'Optional JSON keyed by routing group. Each value accepts probability 0–100 and intensity off, low, standard, or high. Unlisted groups use the global probability.':
      'JSON tùy chọn với khóa là nhóm định tuyến. Mỗi mục nhận xác suất 0–100 và cường độ off, low, standard hoặc high. Nhóm chưa liệt kê dùng xác suất toàn cục.',
  },
}

for (const [locale, translations] of Object.entries(
  requestReviewTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const assistantReviewSummaryTranslations = {
  en: {
    'No completed assistant review is available yet.':
      'No completed assistant review is available yet.',
    Profiles: 'Profiles',
    'Pending support': 'Pending support',
    'Clicks / conversations / approvals': 'Clicks / conversations / approvals',
    Commerce: 'Commerce',
    'Chat users': 'Chat users',
    'Paid users': 'Paid users',
    'Conversion rate': 'Conversion rate',
    Refunds: 'Refunds',
    'Security audit': 'Security audit',
    Matches: 'Matches',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.',
    assistant_review: 'Assistant review',
    Decision: 'Decision',
  },
  zh: {
    'No completed assistant review is available yet.': '暂无已完成的 AI 复盘。',
    Profiles: '用户画像',
    'Pending support': '待处理客服',
    'Clicks / conversations / approvals': '点击 / 对话 / 推荐信 / 批准',
    Commerce: '业务转化',
    'Chat users': '对话用户',
    'Paid users': '付费用户',
    'Conversion rate': '转化率',
    Refunds: '退款',
    'Security audit': '安全审查',
    Matches: '匹配数',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      '本次复盘目前只有聚合指标；后端升级后会显示更详细的安全与业务数据。',
    assistant_review: 'AI 复盘',
    Decision: '判定',
  },
  'zh-TW': {
    'No completed assistant review is available yet.': '尚無已完成的 AI 複盤。',
    Profiles: '使用者畫像',
    'Pending support': '待處理客服',
    'Clicks / conversations / approvals': '點擊 / 對話 / 推薦信 / 核准',
    Commerce: '業務轉化',
    'Chat users': '對話使用者',
    'Paid users': '付費使用者',
    'Conversion rate': '轉化率',
    Refunds: '退款',
    'Security audit': '安全稽核',
    Matches: '匹配數',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      '本次複盤目前只有彙總指標；後端升級後會顯示更詳細的安全與業務資料。',
    assistant_review: 'AI 複盤',
    Decision: '判定',
  },
  fr: {
    'No completed assistant review is available yet.':
      'Aucune revue de l’assistant terminée pour le moment.',
    Profiles: 'Profils',
    'Pending support': 'Support en attente',
    'Clicks / conversations / approvals': 'Clics / conversations / validations',
    Commerce: 'Activité commerciale',
    'Chat users': 'Utilisateurs du chat',
    'Paid users': 'Utilisateurs payants',
    'Conversion rate': 'Taux de conversion',
    Refunds: 'Remboursements',
    'Security audit': 'Audit de sécurité',
    Matches: 'Correspondances',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      'Cette revue ne contient que des indicateurs agrégés ; les détails sécurité et commerce apparaîtront après la mise à jour du backend.',
    assistant_review: 'Revue de l’assistant',
    Decision: 'Décision',
  },
  ja: {
    'No completed assistant review is available yet.':
      '完了したアシスタントレビューはまだありません。',
    Profiles: 'ユーザープロファイル',
    'Pending support': '対応待ちサポート',
    'Clicks / conversations / approvals': 'クリック / 会話 / 推薦 / 承認',
    Commerce: '利用・購入状況',
    'Chat users': 'チャット利用者',
    'Paid users': '有料利用者',
    'Conversion rate': '転換率',
    Refunds: '返金',
    'Security audit': 'セキュリティ監査',
    Matches: '一致数',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      '今回のレビューは集計指標のみです。バックエンド更新後にセキュリティと利用状況の詳細が表示されます。',
    assistant_review: 'アシスタントレビュー',
    Decision: '判定',
  },
  ru: {
    'No completed assistant review is available yet.':
      'Завершённых проверок помощника пока нет.',
    Profiles: 'Профили',
    'Pending support': 'Ожидающая поддержка',
    'Clicks / conversations / approvals':
      'Клики / диалоги / рекомендации / одобрения',
    Commerce: 'Коммерция',
    'Chat users': 'Пользователи чата',
    'Paid users': 'Платящие пользователи',
    'Conversion rate': 'Конверсия',
    Refunds: 'Возвраты',
    'Security audit': 'Аудит безопасности',
    Matches: 'Совпадения',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      'Эта проверка содержит только агрегированные показатели; подробности безопасности и коммерции появятся после обновления backend.',
    assistant_review: 'Проверка помощника',
    Decision: 'Решение',
  },
  vi: {
    'No completed assistant review is available yet.':
      'Chưa có phiên đánh giá trợ lý nào hoàn tất.',
    Profiles: 'Hồ sơ',
    'Pending support': 'Hỗ trợ đang chờ',
    'Clicks / conversations / approvals':
      'Lượt nhấp / hội thoại / đề xuất / phê duyệt',
    Commerce: 'Thương mại',
    'Chat users': 'Người dùng trò chuyện',
    'Paid users': 'Người dùng trả phí',
    'Conversion rate': 'Tỷ lệ chuyển đổi',
    Refunds: 'Hoàn tiền',
    'Security audit': 'Kiểm toán bảo mật',
    Matches: 'Lượt khớp',
    'This run contains aggregate assistant metrics only. Detailed security and commerce sections will appear after the backend update.':
      'Lần đánh giá này chỉ có chỉ số tổng hợp; chi tiết bảo mật và thương mại sẽ xuất hiện sau khi backend được cập nhật.',
    assistant_review: 'Đánh giá trợ lý',
    Decision: 'Kết luận',
  },
}

for (const [locale, translations] of Object.entries(
  assistantReviewSummaryTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const sidebarPreferencesTranslations = {
  en: {
    'Sidebar density': 'Sidebar density',
    'Default page': 'Default page',
    'Use system default': 'Use system default',
    'Move section up': 'Move section up',
    'Move section down': 'Move section down',
    'Move item up': 'Move item up',
    'Move item down': 'Move item down',
    'Start here and review available work':
      'Start here and review available work',
    'Service guide and onboarding': 'Service guide and onboarding',
    'Browse shared channels': 'Browse shared channels',
    'Review pending tasks and notices': 'Review pending tasks and notices',
    'Challenges and community work': 'Challenges and community work',
    'Open challenges': 'Open challenges',
    'Review and manage conversations': 'Review and manage conversations',
    'Open a chat session': 'Open a chat session',
    'System overview': 'System overview',
    'Create and review images': 'Create and review images',
    'Balance and payment management': 'Balance and payment management',
    'Administrative tools': 'Administrative tools',
    'Manage channels': 'Manage channels',
    'Manage models': 'Manage models',
    'Manage users': 'Manage users',
    'Manage redemption codes': 'Manage redemption codes',
    'Manage discount codes': 'Manage discount codes',
    'Manage subscriptions': 'Manage subscriptions',
    'Review platform finances': 'Review platform finances',
    'Inspect system information': 'Inspect system information',
    'Configure the service': 'Configure the service',
  },
  zh: {
    'Sidebar density': '侧栏密度',
    'Default page': '默认页面',
    'Use system default': '使用系统默认',
    'Move section up': '上移分组',
    'Move section down': '下移分组',
    'Move item up': '上移项目',
    'Move item down': '下移项目',
    'Start here and review available work': '从这里开始，查看可用功能',
    'Service guide and onboarding': '服务向导与入门引导',
    'Browse shared channels': '浏览共享渠道',
    'Review pending tasks and notices': '查看待办任务和通知',
    'Challenges and community work': '挑战与社区任务',
    'Open challenges': '公开挑战',
    'Review and manage conversations': '查看和管理对话',
    'Open a chat session': '打开聊天会话',
    'System overview': '系统概览',
    'Create and review images': '创建和查看图片',
    'Balance and payment management': '余额与支付管理',
    'Administrative tools': '管理工具',
    'Manage channels': '管理渠道',
    'Manage models': '管理模型',
    'Manage users': '管理用户',
    'Manage redemption codes': '管理兑换码',
    'Manage discount codes': '管理优惠码',
    'Manage subscriptions': '管理订阅',
    'Review platform finances': '查看平台财务',
    'Inspect system information': '查看系统信息',
    'Configure the service': '配置服务',
  },
  'zh-TW': {
    'Sidebar density': '側欄密度',
    'Default page': '預設頁面',
    'Use system default': '使用系統預設',
    'Move section up': '上移分組',
    'Move section down': '下移分組',
    'Move item up': '上移項目',
    'Move item down': '下移項目',
    'Start here and review available work': '從這裡開始，查看可用功能',
    'Service guide and onboarding': '服務導覽與入門引導',
    'Browse shared channels': '瀏覽共享渠道',
    'Review pending tasks and notices': '查看待辦任務與通知',
    'Challenges and community work': '挑戰與社群任務',
    'Open challenges': '公開挑戰',
    'Review and manage conversations': '查看與管理對話',
    'Open a chat session': '開啟聊天會話',
    'System overview': '系統概覽',
    'Create and review images': '建立與查看圖片',
    'Balance and payment management': '餘額與付款管理',
    'Administrative tools': '管理工具',
    'Manage channels': '管理渠道',
    'Manage models': '管理模型',
    'Manage users': '管理使用者',
    'Manage redemption codes': '管理兌換碼',
    'Manage discount codes': '管理折扣碼',
    'Manage subscriptions': '管理訂閱',
    'Review platform finances': '查看平台財務',
    'Inspect system information': '查看系統資訊',
    'Configure the service': '設定服務',
  },
  fr: {
    'Sidebar density': 'Densité de la barre latérale',
    'Default page': 'Page par défaut',
    'Use system default': 'Utiliser la valeur système',
    'Move section up': 'Monter la section',
    'Move section down': 'Descendre la section',
    'Move item up': 'Monter l’élément',
    'Move item down': 'Descendre l’élément',
    'Start here and review available work':
      'Commencer ici et voir le travail disponible',
    'Service guide and onboarding': 'Guide du service et démarrage',
    'Browse shared channels': 'Parcourir les canaux partagés',
    'Review pending tasks and notices':
      'Voir les tâches et notifications en attente',
    'Challenges and community work': 'Défis et travail communautaire',
    'Open challenges': 'Défis ouverts',
    'Review and manage conversations': 'Voir et gérer les conversations',
    'Open a chat session': 'Ouvrir une session de chat',
    'System overview': 'Vue d’ensemble du système',
    'Create and review images': 'Créer et consulter des images',
    'Balance and payment management': 'Solde et paiements',
    'Administrative tools': 'Outils d’administration',
    'Manage channels': 'Gérer les canaux',
    'Manage models': 'Gérer les modèles',
    'Manage users': 'Gérer les utilisateurs',
    'Manage redemption codes': 'Gérer les codes de rachat',
    'Manage discount codes': 'Gérer les codes promotionnels',
    'Manage subscriptions': 'Gérer les abonnements',
    'Review platform finances': 'Voir les finances de la plateforme',
    'Inspect system information': 'Consulter les informations système',
    'Configure the service': 'Configurer le service',
  },
  ja: {
    'Sidebar density': 'サイドバーの密度',
    'Default page': '既定のページ',
    'Use system default': 'システム既定を使用',
    'Move section up': 'セクションを上へ移動',
    'Move section down': 'セクションを下へ移動',
    'Move item up': '項目を上へ移動',
    'Move item down': '項目を下へ移動',
    'Start here and review available work':
      'ここから始めて利用可能な機能を確認',
    'Service guide and onboarding': 'サービスガイドと初期設定',
    'Browse shared channels': '共有チャンネルを閲覧',
    'Review pending tasks and notices': '保留中のタスクと通知を確認',
    'Challenges and community work': 'チャレンジとコミュニティの作業',
    'Open challenges': '公開チャレンジ',
    'Review and manage conversations': '会話を確認・管理',
    'Open a chat session': 'チャットセッションを開く',
    'System overview': 'システム概要',
    'Create and review images': '画像を作成・確認',
    'Balance and payment management': '残高と支払いの管理',
    'Administrative tools': '管理ツール',
    'Manage channels': 'チャンネルを管理',
    'Manage models': 'モデルを管理',
    'Manage users': 'ユーザーを管理',
    'Manage redemption codes': '引き換えコードを管理',
    'Manage discount codes': '割引コードを管理',
    'Manage subscriptions': 'サブスクリプションを管理',
    'Review platform finances': 'プラットフォームの財務を確認',
    'Inspect system information': 'システム情報を確認',
    'Configure the service': 'サービスを設定',
  },
  ru: {
    'Sidebar density': 'Плотность боковой панели',
    'Default page': 'Страница по умолчанию',
    'Use system default': 'Использовать системное значение',
    'Move section up': 'Переместить раздел вверх',
    'Move section down': 'Переместить раздел вниз',
    'Move item up': 'Переместить пункт вверх',
    'Move item down': 'Переместить пункт вниз',
    'Start here and review available work':
      'Начните здесь и просмотрите доступные функции',
    'Service guide and onboarding':
      'Руководство по сервису и начальная настройка',
    'Browse shared channels': 'Просмотреть общие каналы',
    'Review pending tasks and notices':
      'Просмотреть ожидающие задачи и уведомления',
    'Challenges and community work': 'Задания и работа сообщества',
    'Open challenges': 'Открытые задания',
    'Review and manage conversations': 'Просматривать и управлять диалогами',
    'Open a chat session': 'Открыть чат',
    'System overview': 'Обзор системы',
    'Create and review images': 'Создавать и просматривать изображения',
    'Balance and payment management': 'Баланс и платежи',
    'Administrative tools': 'Инструменты администрирования',
    'Manage channels': 'Управлять каналами',
    'Manage models': 'Управлять моделями',
    'Manage users': 'Управлять пользователями',
    'Manage redemption codes': 'Управлять кодами погашения',
    'Manage discount codes': 'Управлять кодами скидок',
    'Manage subscriptions': 'Управлять подписками',
    'Review platform finances': 'Просмотреть финансы платформы',
    'Inspect system information': 'Просмотреть сведения о системе',
    'Configure the service': 'Настроить сервис',
  },
  vi: {
    'Sidebar density': 'Mật độ thanh bên',
    'Default page': 'Trang mặc định',
    'Use system default': 'Dùng mặc định của hệ thống',
    'Move section up': 'Đưa mục lên',
    'Move section down': 'Đưa mục xuống',
    'Move item up': 'Đưa mục con lên',
    'Move item down': 'Đưa mục con xuống',
    'Start here and review available work':
      'Bắt đầu tại đây và xem các tính năng khả dụng',
    'Service guide and onboarding': 'Hướng dẫn dịch vụ và bắt đầu sử dụng',
    'Browse shared channels': 'Duyệt các kênh được chia sẻ',
    'Review pending tasks and notices': 'Xem công việc và thông báo đang chờ',
    'Challenges and community work': 'Thử thách và công việc cộng đồng',
    'Open challenges': 'Thử thách mở',
    'Review and manage conversations': 'Xem và quản lý hội thoại',
    'Open a chat session': 'Mở phiên trò chuyện',
    'System overview': 'Tổng quan hệ thống',
    'Create and review images': 'Tạo và xem hình ảnh',
    'Balance and payment management': 'Quản lý số dư và thanh toán',
    'Administrative tools': 'Công cụ quản trị',
    'Manage channels': 'Quản lý kênh',
    'Manage models': 'Quản lý model',
    'Manage users': 'Quản lý người dùng',
    'Manage redemption codes': 'Quản lý mã đổi thưởng',
    'Manage discount codes': 'Quản lý mã giảm giá',
    'Manage subscriptions': 'Quản lý gói đăng ký',
    'Review platform finances': 'Xem tài chính nền tảng',
    'Inspect system information': 'Xem thông tin hệ thống',
    'Configure the service': 'Cấu hình dịch vụ',
  },
}

for (const [locale, translations] of Object.entries(
  sidebarPreferencesTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const weeklyDiscountTranslations = {
  en: {
    'Weekly discount claimed': 'Weekly discount claimed',
    'Unable to claim weekly discount': 'Unable to claim weekly discount',
    "This week's decision is used": "This week's decision is used",
    'Claim discount code': 'Claim discount code',
    'Code hidden': 'Code hidden',
    'Discount code copied': 'Discount code copied',
    'Weekly recharge discount': 'Weekly recharge discount',
    'One claim per UTC week': 'One claim per UTC week',
    'Profit is unavailable for a payment-method filter':
      'Profit is unavailable for a payment-method filter',
    'Usage is unavailable for a payment-method filter':
      'Usage is unavailable for a payment-method filter',
  },
  fr: {
    'Weekly discount claimed': 'Remise hebdomadaire réclamée',
    'Unable to claim weekly discount':
      'Impossible de réclamer la remise hebdomadaire',
    "This week's decision is used": 'La décision de cette semaine est utilisée',
    'Claim discount code': 'Réclamer le code promo',
    'Code hidden': 'Code masqué',
    'Discount code copied': 'Code promo copié',
    'Weekly recharge discount': 'Remise de recharge hebdomadaire',
    'One claim per UTC week': 'Une réclamation par semaine UTC',
    'Profit is unavailable for a payment-method filter':
      'Le bénéfice est indisponible avec un filtre de moyen de paiement',
    'Usage is unavailable for a payment-method filter':
      "L'utilisation est indisponible avec un filtre de moyen de paiement",
  },
  ja: {
    'Weekly discount claimed': '毎週割引を受け取りました',
    'Unable to claim weekly discount': '毎週割引を受け取れません',
    "This week's decision is used": '今週の判定は使用済みです',
    'Claim discount code': '割引コードを受け取る',
    'Code hidden': 'コードは非表示です',
    'Discount code copied': '割引コードをコピーしました',
    'Weekly recharge discount': '毎週のチャージ割引',
    'One claim per UTC week': 'UTC週ごとに1回まで',
    'Profit is unavailable for a payment-method filter':
      '支払方法フィルター使用時は利益を表示できません',
    'Usage is unavailable for a payment-method filter':
      '支払方法フィルター使用時は使用量を表示できません',
  },
  ru: {
    'Weekly discount claimed': 'Еженедельная скидка получена',
    'Unable to claim weekly discount':
      'Не удалось получить еженедельную скидку',
    "This week's decision is used": 'Решение этой недели уже использовано',
    'Claim discount code': 'Получить код скидки',
    'Code hidden': 'Код скрыт',
    'Discount code copied': 'Код скидки скопирован',
    'Weekly recharge discount': 'Еженедельная скидка на пополнение',
    'One claim per UTC week': 'Один раз за неделю UTC',
    'Profit is unavailable for a payment-method filter':
      'При фильтре по способу оплаты прибыль недоступна',
    'Usage is unavailable for a payment-method filter':
      'При фильтре по способу оплаты использование недоступно',
  },
  vi: {
    'Weekly discount claimed': 'Đã nhận ưu đãi hàng tuần',
    'Unable to claim weekly discount': 'Không thể nhận ưu đãi hàng tuần',
    "This week's decision is used": 'Đã dùng quyết định của tuần này',
    'Claim discount code': 'Nhận mã giảm giá',
    'Code hidden': 'Mã đang ẩn',
    'Discount code copied': 'Đã sao chép mã giảm giá',
    'Weekly recharge discount': 'Ưu đãi nạp tiền hàng tuần',
    'One claim per UTC week': 'Mỗi tuần UTC chỉ nhận một lần',
    'Profit is unavailable for a payment-method filter':
      'Không thể tính lợi nhuận khi lọc theo phương thức thanh toán',
    'Usage is unavailable for a payment-method filter':
      'Không thể xác định mức sử dụng khi lọc theo phương thức thanh toán',
  },
  'zh-TW': {
    'Weekly discount claimed': '每週優惠已領取',
    'Unable to claim weekly discount': '無法領取每週優惠',
    "This week's decision is used": '本週評估已使用',
    'Claim discount code': '領取優惠碼',
    'Code hidden': '優惠碼暫不可見',
    'Discount code copied': '優惠碼已複製',
    'Weekly recharge discount': '每週充值折扣',
    'One claim per UTC week': '每個 UTC 週限領一次',
    'Profit is unavailable for a payment-method filter':
      '套用付款方式篩選時無法提供利潤',
    'Usage is unavailable for a payment-method filter':
      '套用付款方式篩選時無法提供用量',
  },
  zh: {
    'Weekly discount claimed': '每周优惠已领取',
    'Unable to claim weekly discount': '无法领取每周优惠',
    "This week's decision is used": '本周评估已使用',
    'Claim discount code': '领取优惠码',
    'Code hidden': '优惠码暂不可见',
    'Discount code copied': '优惠码已复制',
    'Weekly recharge discount': '每周充值折扣',
    'One claim per UTC week': '每个 UTC 周限领一次',
    'Profit is unavailable for a payment-method filter':
      '按支付方式筛选时无法计算利润',
    'Usage is unavailable for a payment-method filter':
      '按支付方式筛选时无法归因用量',
  },
}

for (const [locale, translations] of Object.entries(
  weeklyDiscountTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const ipAccessRoutingTranslations = {
  en: {
    'IP & Region Routing': 'IP & Region Routing',
    'At least one routing rule is required.':
      'At least one routing rule is required.',
    'Routing rules cannot exceed 16384 bytes.':
      'Routing rules cannot exceed 16384 bytes.',
    'Routing rules are invalid.': 'Routing rules are invalid.',
    'First matching rule wins': 'First matching rule wins',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.',
    'Keep management access first': 'Keep management access first',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.',
    'Routing rules': 'Routing rules',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.',
  },
  zh: {
    'IP & Region Routing': 'IP 与地区路由',
    'At least one routing rule is required.': '至少需要一条路由规则。',
    'Routing rules cannot exceed 16384 bytes.': '路由规则不能超过 16384 字节。',
    'Routing rules are invalid.': '路由规则无效。',
    'First matching rule wins': '首条匹配规则生效',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      '规则从上到下执行。direct 允许请求，reject 拒绝请求；未命中任何规则的请求默认允许。',
    'Keep management access first': '先保留管理访问',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      '请将可信管理 IP 的 direct 规则放在宽泛的 reject 规则之前，避免把自己锁在系统外。',
    'Routing rules': '路由规则',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      '每行填写一条 daed 风格规则。支持 dip(IP、CIDR、geoip:xx、geoip:private)、l4proto(tcp) 和 dport(port)；使用 # 添加注释。',
  },
  'zh-TW': {
    'IP & Region Routing': 'IP 與地區路由',
    'At least one routing rule is required.': '至少需要一條路由規則。',
    'Routing rules cannot exceed 16384 bytes.':
      '路由規則不能超過 16384 位元組。',
    'Routing rules are invalid.': '路由規則無效。',
    'First matching rule wins': '首條符合規則生效',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      '規則由上而下執行。direct 允許請求，reject 拒絕請求；未符合任何規則的請求預設允許。',
    'Keep management access first': '先保留管理存取',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      '請將可信管理 IP 的 direct 規則放在廣泛的 reject 規則之前，避免將自己鎖在系統外。',
    'Routing rules': '路由規則',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      '每行填寫一條 daed 風格規則。支援 dip(IP、CIDR、geoip:xx、geoip:private)、l4proto(tcp) 和 dport(port)；使用 # 加入註解。',
  },
  fr: {
    'IP & Region Routing': 'Routage IP et régional',
    'At least one routing rule is required.':
      'Au moins une règle de routage est requise.',
    'Routing rules cannot exceed 16384 bytes.':
      'Les règles de routage ne peuvent pas dépasser 16 384 octets.',
    'Routing rules are invalid.': 'Les règles de routage sont invalides.',
    'First matching rule wins': 'La première règle correspondante s’applique',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      'Les règles sont évaluées de haut en bas. direct autorise la requête, reject la bloque et les requêtes sans correspondance sont autorisées.',
    'Keep management access first':
      'Préserver d’abord l’accès d’administration',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      'Placez les règles direct des IP d’administration approuvées avant les règles reject générales afin de ne pas bloquer votre propre accès.',
    'Routing rules': 'Règles de routage',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      'Utilisez une règle de style daed par ligne. Prédicats pris en charge : dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp) et dport(port). Utilisez # pour les commentaires.',
  },
  ja: {
    'IP & Region Routing': 'IP・地域ルーティング',
    'At least one routing rule is required.':
      '少なくとも1つのルーティングルールが必要です。',
    'Routing rules cannot exceed 16384 bytes.':
      'ルーティングルールは16384バイト以内にしてください。',
    'Routing rules are invalid.': 'ルーティングルールが無効です。',
    'First matching rule wins': '最初に一致したルールを適用',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      'ルールは上から順に評価されます。direct はリクエストを許可し、reject は拒否します。どのルールにも一致しないリクエストは許可されます。',
    'Keep management access first': '管理アクセスを先に確保',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      'ロックアウトを防ぐため、信頼済み管理IPの direct ルールを広範な reject ルールより上に配置してください。',
    'Routing rules': 'ルーティングルール',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      '1行に1つの daed 形式ルールを記述します。対応条件は dip(IP, CIDR, geoip:xx, geoip:private)、l4proto(tcp)、dport(port) です。コメントには # を使用します。',
  },
  ru: {
    'IP & Region Routing': 'Маршрутизация по IP и регионам',
    'At least one routing rule is required.':
      'Требуется хотя бы одно правило маршрутизации.',
    'Routing rules cannot exceed 16384 bytes.':
      'Правила маршрутизации не должны превышать 16 384 байта.',
    'Routing rules are invalid.': 'Правила маршрутизации недействительны.',
    'First matching rule wins': 'Применяется первое совпавшее правило',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      'Правила проверяются сверху вниз. direct разрешает запрос, reject блокирует его, а запросы без совпадений разрешаются.',
    'Keep management access first': 'Сначала сохраните административный доступ',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      'Поместите правила direct для доверенных административных IP-адресов выше общих правил reject, чтобы не заблокировать себе доступ.',
    'Routing rules': 'Правила маршрутизации',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      'Указывайте по одному правилу в стиле daed на строку. Поддерживаются dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp) и dport(port). Для комментариев используйте #.',
  },
  vi: {
    'IP & Region Routing': 'Định tuyến IP và khu vực',
    'At least one routing rule is required.':
      'Cần ít nhất một quy tắc định tuyến.',
    'Routing rules cannot exceed 16384 bytes.':
      'Quy tắc định tuyến không được vượt quá 16384 byte.',
    'Routing rules are invalid.': 'Quy tắc định tuyến không hợp lệ.',
    'First matching rule wins': 'Áp dụng quy tắc khớp đầu tiên',
    'Rules run from top to bottom. direct allows the request, reject blocks it, and requests that match no rule are allowed.':
      'Quy tắc chạy từ trên xuống. direct cho phép yêu cầu, reject chặn yêu cầu; yêu cầu không khớp quy tắc nào sẽ được phép.',
    'Keep management access first': 'Ưu tiên giữ quyền truy cập quản trị',
    'Put direct rules for trusted management IPs above broad reject rules so you do not lock yourself out.':
      'Đặt quy tắc direct cho IP quản trị tin cậy phía trên các quy tắc reject rộng để tránh tự khóa quyền truy cập.',
    'Routing rules': 'Quy tắc định tuyến',
    'Use one daed-style rule per line. Supported matchers: dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp), and dport(port). Use # for comments.':
      'Mỗi dòng dùng một quy tắc kiểu daed. Hỗ trợ dip(IP, CIDR, geoip:xx, geoip:private), l4proto(tcp) và dport(port). Dùng # cho chú thích.',
  },
}

for (const [locale, translations] of Object.entries(
  ipAccessRoutingTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

const remainingStaticKeyTranslations = {
  en: {
    'Add the public description and confirm that this channel can be shared.':
      'Add the public description and confirm that this channel can be shared.',
    Approve: 'Approve',
    Canvas: 'Canvas',
    'Community rankings': 'Community rankings',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      'Complete the full channel configuration below. The submission remains pending until an administrator approves it.',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.',
    Contributor: 'Contributor',
    'I confirm that these credentials are authorized for this shared channel.':
      'I confirm that these credentials are authorized for this shared channel.',
    Inspector: 'Inspector',
    'Settings saved': 'Settings saved',
    'Sharing information': 'Sharing information',
    'This information is shown in the public channel market after approval.':
      'This information is shown in the public channel market after approval.',
    'Top-up amount': 'Top-up amount',
    'Unable to update payment method': 'Unable to update payment method',
    'View user in user management': 'View user in user management',
  },
  zh: {
    'Add the public description and confirm that this channel can be shared.':
      '添加公开说明，并确认此渠道可以共享。',
    Approve: '批准',
    Canvas: '画布',
    'Community rankings': '社区排名',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      '请在下方完成完整渠道配置。提交内容在管理员批准前将保持待审核状态。',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      '为各分组配置警告和确认次数（1–3 次）。最醒目的警告请使用弹窗模式。',
    Contributor: '贡献者',
    'I confirm that these credentials are authorized for this shared channel.':
      '我确认这些凭证已获授权用于此共享渠道。',
    Inspector: '检查器',
    'Settings saved': '设置已保存',
    'Sharing information': '共享信息',
    'This information is shown in the public channel market after approval.':
      '批准后，此信息将显示在公开渠道市场中。',
    'Top-up amount': '充值金额',
    'Unable to update payment method': '无法更新支付方式',
    'View user in user management': '在用户管理中查看用户',
  },
  'zh-TW': {
    'Add the public description and confirm that this channel can be shared.':
      '新增公開說明，並確認此渠道可以共享。',
    Approve: '核准',
    Canvas: '畫布',
    'Community rankings': '社群排名',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      '請在下方完成完整渠道設定。提交內容在管理員核准前將維持待審核狀態。',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      '為各分組設定警告和確認次數（1–3 次）。最醒目的警告請使用彈窗模式。',
    Contributor: '貢獻者',
    'I confirm that these credentials are authorized for this shared channel.':
      '我確認這些憑證已獲授權用於此共享渠道。',
    Inspector: '檢查器',
    'Settings saved': '設定已儲存',
    'Sharing information': '共享資訊',
    'This information is shown in the public channel market after approval.':
      '核准後，此資訊將顯示在公開渠道市場中。',
    'Top-up amount': '充值金額',
    'Unable to update payment method': '無法更新付款方式',
    'View user in user management': '在使用者管理中查看使用者',
  },
  fr: {
    'Add the public description and confirm that this channel can be shared.':
      'Ajoutez la description publique et confirmez que ce canal peut être partagé.',
    Approve: 'Approuver',
    Canvas: 'Canevas',
    'Community rankings': 'Classements de la communauté',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      'Renseignez toute la configuration du canal ci-dessous. La soumission reste en attente jusqu’à son approbation par un administrateur.',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      'Configurez les avertissements par groupe et le nombre de confirmations (1 à 3). Utilisez le mode modal pour l’avertissement le plus visible.',
    Contributor: 'Contributeur',
    'I confirm that these credentials are authorized for this shared channel.':
      'Je confirme que ces identifiants sont autorisés pour ce canal partagé.',
    Inspector: 'Inspecteur',
    'Settings saved': 'Paramètres enregistrés',
    'Sharing information': 'Informations de partage',
    'This information is shown in the public channel market after approval.':
      'Ces informations apparaîtront sur le marché public des canaux après approbation.',
    'Top-up amount': 'Montant de la recharge',
    'Unable to update payment method':
      'Impossible de mettre à jour le moyen de paiement',
    'View user in user management':
      'Voir l’utilisateur dans la gestion des utilisateurs',
  },
  ja: {
    'Add the public description and confirm that this channel can be shared.':
      '公開説明を追加し、このチャネルを共有できることを確認してください。',
    Approve: '承認',
    Canvas: 'キャンバス',
    'Community rankings': 'コミュニティランキング',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      '以下のチャネル設定をすべて入力してください。管理者が承認するまで申請は保留状態になります。',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      'グループごとの警告と確認回数（1～3回）を設定します。最も目立たせる警告にはモーダルを使用してください。',
    Contributor: 'コントリビューター',
    'I confirm that these credentials are authorized for this shared channel.':
      'これらの認証情報がこの共有チャネルでの使用を許可されていることを確認します。',
    Inspector: 'インスペクター',
    'Settings saved': '設定を保存しました',
    'Sharing information': '共有情報',
    'This information is shown in the public channel market after approval.':
      '承認後、この情報は公開チャネルマーケットに表示されます。',
    'Top-up amount': 'チャージ金額',
    'Unable to update payment method': '支払方法を更新できません',
    'View user in user management': 'ユーザー管理でユーザーを表示',
  },
  ru: {
    'Add the public description and confirm that this channel can be shared.':
      'Добавьте публичное описание и подтвердите, что этот канал можно использовать совместно.',
    Approve: 'Одобрить',
    Canvas: 'Холст',
    'Community rankings': 'Рейтинг сообщества',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      'Заполните полную конфигурацию канала ниже. Заявка останется на рассмотрении до одобрения администратором.',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      'Настройте предупреждения для групп и число подтверждений (1–3). Для наиболее заметного предупреждения используйте модальное окно.',
    Contributor: 'Участник',
    'I confirm that these credentials are authorized for this shared channel.':
      'Я подтверждаю, что эти учётные данные разрешено использовать для общего канала.',
    Inspector: 'Инспектор',
    'Settings saved': 'Настройки сохранены',
    'Sharing information': 'Сведения для публикации',
    'This information is shown in the public channel market after approval.':
      'После одобрения эти сведения появятся в каталоге общедоступных каналов.',
    'Top-up amount': 'Сумма пополнения',
    'Unable to update payment method': 'Не удалось обновить способ оплаты',
    'View user in user management': 'Открыть пользователя в разделе управления',
  },
  vi: {
    'Add the public description and confirm that this channel can be shared.':
      'Thêm mô tả công khai và xác nhận rằng kênh này có thể được chia sẻ.',
    Approve: 'Phê duyệt',
    Canvas: 'Khung vẽ',
    'Community rankings': 'Xếp hạng cộng đồng',
    'Complete the full channel configuration below. The submission remains pending until an administrator approves it.':
      'Hoàn tất toàn bộ cấu hình kênh bên dưới. Nội dung gửi sẽ ở trạng thái chờ cho đến khi quản trị viên phê duyệt.',
    'Configure per-group warnings and acknowledgement count (1–3). Use modal for the most prominent warning.':
      'Cấu hình cảnh báo theo nhóm và số lần xác nhận (1–3). Dùng hộp thoại cho cảnh báo nổi bật nhất.',
    Contributor: 'Người đóng góp',
    'I confirm that these credentials are authorized for this shared channel.':
      'Tôi xác nhận các thông tin xác thực này được phép dùng cho kênh chia sẻ.',
    Inspector: 'Trình kiểm tra',
    'Settings saved': 'Đã lưu cài đặt',
    'Sharing information': 'Thông tin chia sẻ',
    'This information is shown in the public channel market after approval.':
      'Thông tin này sẽ hiển thị trên chợ kênh công khai sau khi được phê duyệt.',
    'Top-up amount': 'Số tiền nạp',
    'Unable to update payment method':
      'Không thể cập nhật phương thức thanh toán',
    'View user in user management':
      'Xem người dùng trong phần quản lý người dùng',
  },
}

for (const [locale, translations] of Object.entries(
  remainingStaticKeyTranslations
)) {
  Object.assign(newKeys[locale], translations)
}

async function main() {
  let totalAdded = 0
  for (const [locale, translations] of Object.entries(newKeys)) {
    const filePath = path.join(LOCALES_DIR, `${locale}.json`)
    const json = JSON.parse(await fs.readFile(filePath, 'utf8'))
    let count = 0
    if ('Routing rules cannot exceed 16384 characters.' in json.translation) {
      delete json.translation['Routing rules cannot exceed 16384 characters.']
      count++
    }
    for (const [key, value] of Object.entries(translations)) {
      if (json.translation[key] !== value) {
        json.translation[key] = value
        count++
      }
    }
    if (count > 0) {
      json.translation = Object.fromEntries(
        Object.entries(json.translation).sort(([a], [b]) => a.localeCompare(b))
      )
      await fs.writeFile(filePath, stableStringify(json), 'utf8')
    }
    console.log(`${locale}: ${count} translations applied`)
    totalAdded += count
  }
  console.log(`Total: ${totalAdded} translations applied`)
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
