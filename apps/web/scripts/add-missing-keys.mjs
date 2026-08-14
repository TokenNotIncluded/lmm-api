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
    'Security reviews': 'Security reviews',
    'assistant.security_review': 'Assistant security review',
  },
  zh: {
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
    'Security reviews': '安全巡检',
    'assistant.security_review': '助手安全巡检',
  },
  'zh-TW': {
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
    'Security reviews': '安全巡檢',
    'assistant.security_review': '助手安全巡檢',
  },
  fr: {
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
    'Security reviews': 'Revues de sécurité',
    'assistant.security_review': 'Revue de sécurité de l’assistant',
  },
  ja: {
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
    'Security reviews': 'セキュリティレビュー',
    'assistant.security_review': 'アシスタントのセキュリティレビュー',
  },
  ru: {
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
    'Security reviews': 'Проверки безопасности',
    'assistant.security_review': 'Проверка безопасности ассистента',
  },
  vi: {
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
    'Security reviews': 'Đánh giá bảo mật',
    'assistant.security_review': 'Đánh giá bảo mật của trợ lý',
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

for (const [locale, translations] of Object.entries(todoTranslations)) {
  Object.assign(newKeys[locale], translations)
}

async function main() {
  let totalAdded = 0
  for (const [locale, translations] of Object.entries(newKeys)) {
    const filePath = path.join(LOCALES_DIR, `${locale}.json`)
    const json = JSON.parse(await fs.readFile(filePath, 'utf8'))
    let count = 0
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
