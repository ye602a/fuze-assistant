//! UI-thread locale; background workers and dataset identity remain independent.
use std::cell::Cell;
thread_local! { static LOCALE: Cell<u8> = const { Cell::new(0) }; }
pub fn set(locale: &str) {
    LOCALE.set(match locale {
        "zh-TW" => 1,
        "en-US" => 2,
        _ => 0,
    });
}
pub fn locale() -> &'static str {
    match LOCALE.get() {
        1 => "zh-TW",
        2 => "en-US",
        _ => "zh-CN",
    }
}
pub fn text(s: &str) -> &str {
    let locale = LOCALE.get();
    if locale == 0 {
        return s;
    }
    MESSAGES
        .iter()
        .find(|row| row[0] == s)
        .map(|row| row[locale as usize])
        .unwrap_or(s)
}
// One row is one key, so every supported locale has identical coverage.
const MESSAGES: &[[&str; 3]] = &[
    ["未知", "未知", "Unknown"],
    ["时长", "時長", "Duration"],
    ["FUZE / 对局分享", "FUZE / 對局分享", "FUZE / MATCH SUMMARY"],
    ["缓存结果", "快取結果", "CACHED RESULT"],
    ["对局结果", "對局結果", "RESULT"],
    ["结算强化", "結算增幅", "RESULT AUGMENTS"],
    ["领取顺序未确认 / 推荐使用整体胜率", "領取順序未確認 / 推薦使用整體勝率", "Acquisition order unconfirmed / Overall recommendations"],
    ["未包含候选观察、账号、区服、对局编号或时间戳。", "未包含候選觀察、帳號、區服、對局編號或時間戳。", "No candidate observations, account, region, match ID or timestamp."],
    ["符择 · 局内海克斯参考", "符擇 · 局內海克斯參考", "Fuze · In-game augment reference"],
    ["局内海克斯参考 · 整体胜率", "局內海克斯參考 · 整體勝率", "Augment reference · Overall rates"],
    ["实际领取轮次未确认；与三卡识别独立", "實際領取輪次未確認；與三卡辨識獨立", "Claim round unknown; separate from OCR cards"],
    ["显示 / 隐藏局内海克斯参考", "顯示 / 隱藏局內海克斯參考", "Show / hide in-game augment reference"],
    ["卸载独立 Rust 预览版和开始菜单快捷方式？设置、个人记录、缓存与私有令牌会保留。未知文件不会删除；临时卸载工具由系统临时文件清理。", "解除安裝獨立 Rust 預覽版與開始功能表捷徑？設定、個人記錄、快取與私人權杖保留。未知檔案不會刪除；暫存卸載工具由系統暫存清理。", "Remove Rust Preview and Start menu shortcuts? Settings, records, caches and private tokens are kept. Unknown files stay. The temporary helper is left for system temp cleanup."],
    ["Rust 程序与快捷方式已移除。用户数据、令牌、未识别文件和安装锁文件保留。Electron 稳定版未改动。", "Rust 程式與捷徑已移除。使用者資料、權杖、未知檔案與安裝鎖檔保留。Electron 穩定版未變動。", "Rust program and shortcuts removed. User data, tokens, unknown files and installation lock are kept. Electron stable is unchanged."],
    ["安装完成。开始菜单已有 Fuze Rust Preview 启动与卸载快捷方式。首次启动通过健康检查后确认新版；此包仍为未签名开发预览。", "安裝完成。開始功能表已有 Fuze Rust Preview 啟動與卸載捷徑。首次啟動通過健康檢查後確認新版；仍為未簽章開發預覽。", "Installed. Start menu shortcuts launch or uninstall Fuze Rust Preview. First launch validates health. This remains an unsigned development preview."],
    ["操作未完成：{}\n请关闭 Rust 预览版后重试。未知文件会保留；安装或卸载可能部分完成。Electron 稳定版未改动。", "操作未完成：{}\n請關閉 Rust 預覽版後重試。未知檔案保留；安裝或卸載可能部分完成。Electron 穩定版未變動。", "Operation incomplete: {}\nClose Rust Preview before retrying. Unknown files are kept; installation or removal may be partial. Electron stable is unchanged."],
    ["请求较频繁，请稍后刷新", "請求較頻繁，請稍後重新整理", "Please wait before refreshing again"],
    ["正在刷新，继续显示已有数据", "正在重新整理，繼續顯示現有資料", "Refreshing; keeping existing data visible"],
    ["阶段参考", "階段參考", "Stages"],
    ["整体", "整體", "All"],
    ["第1次", "第1次", "1st"],
    ["第2次", "第2次", "2nd"],
    ["第3次", "第3次", "3rd"],
    ["第4次", "第4次", "4th"],
    ["刷新", "重新整理", "Refresh"],
    ["选取率", "選取率", "Pick"],
    ["样本", "樣本", "N"],
    ["手动参考 · 不改变三卡整体胜率推荐", "手動參考 · 不改變三卡整體勝率推薦", "Manual reference; cards retain overall rates"],
    ["阶段表示选择次序；不是已确认的领取轮次", "階段表示選擇次序；並非已確認的領取輪次", "Choice order, not a confirmed acquisition"],
    ["参考数据暂不可用，可稍后刷新", "參考資料暫不可用，可稍後重新整理", "Reference unavailable; try Refresh later"],
    ["正在读取阶段参考…", "正在讀取階段參考…", "Loading stage reference…"],
    ["数据区服未确认 · 仅展示样本量 ≥ 200", "資料區服未確認 · 僅顯示樣本數 ≥ 200", "Region unconfirmed; at least 200 samples"],
    ["离线示例数据", "離線範例資料", "Offline example data"],
    ["与本地数据补丁不同，仅作参考", "與本機資料版本不同，僅供參考", "Different patch from local data; reference only"],
    ["刷新未完成，保留可用数据", "重新整理未完成，保留可用資料", "Refresh incomplete; retained available data"],
    ["缓存已过期，可手动刷新", "快取已過期，可手動重新整理", "Stale cache; refresh manually"],
    ["使用本地缓存", "使用本機快取", "Using local cache"],
    ["参考数据已更新", "參考資料已更新", "Reference updated"],
    ["此阶段暂无足够样本", "此階段暫無足夠樣本", "Insufficient samples for this stage"],
    ["将当前 Rust 测试包安装到独立用户目录？无需管理员权限，不修改稳定版，不自动启动。安装后可删除解压目录；设置和个人记录独立保留。", "將目前 Rust 測試包安裝至獨立使用者目錄？無需管理員權限，不修改穩定版，不自動啟動。安裝後可刪除解壓目錄；設定與個人記錄獨立保留。", "Install this Rust preview into its separate user directory? No administrator access, stable-app changes or automatic launch. The extracted download can then be removed; settings and records remain separate."],
    ["安装完成。请运行 LOCALAPPDATA/FuzeRustPreview/releases/Start-Fuze-Rust.cmd。首次启动通过健康检查后确认新版；此包仍为未签名开发预览。", "安裝完成。請執行 LOCALAPPDATA/FuzeRustPreview/releases/Start-Fuze-Rust.cmd。首次啟動通過健康檢查後確認新版；此包仍為未簽署開發預覽。", "Installed. Run LOCALAPPDATA/FuzeRustPreview/releases/Start-Fuze-Rust.cmd. The first healthy launch commits this version. This remains an unsigned development preview."],
    ["当前版本已安装，启动入口已检查。没有重复安装或启动程序。", "目前版本已安裝，啟動入口已檢查。沒有重複安裝或啟動程式。", "This version is already installed and its entry point was checked. Nothing was reinstalled or launched."],
    ["导出当前对局（匿名）", "匯出匿名對局", "Export anonymous match"],
    ["匿名导出已保存到本机。", "匿名匯出已儲存於本機。", "Anonymous export saved locally."],
    ["不含账号、区服、时间与对局标识。", "不含帳號、區服、時間與對局識別碼。", "No account, region, timestamps or match IDs."],
    ["等级辅助调度", "等級輔助排程", "Level-aware scheduling"],
    ["显示三卡推荐", "顯示三卡推薦", "Show three-card suggestions"],
    ["当前等级", "目前等級", "Current level"],
    ["仅辅助调度，推荐仍用整体胜率", "僅輔助排程，推薦仍使用整體勝率", "Scheduling hint only; overall win rates"],
    ["等级暂不可用，继续低频检测；不会停止识别。", "等級暫時不可用，繼續低頻偵測。", "Level unavailable; periodic recognition remains active."],
    ["等级调度已关闭，使用常规检测。", "等級排程已關閉，使用一般偵測。", "Level scheduling is off; regular recognition remains active."],
    ["符择 · Rust 更新", "符擇 · Rust 更新", "FUZE · Rust update"],
    ["取消", "取消", "Cancel"],
    ["正在取消 · 等待当前请求结束，旧版本保留", "正在取消 · 等待目前請求結束，保留舊版", "Cancelling · Waiting for the current request; previous version retained"],
    ["等待确认 · 随时可以取消，旧版本保留", "等待確認 · 可隨時取消，保留舊版", "Waiting for confirmation · You can cancel; previous version retained"],
    ["正在检查固定 Rust 频道", "正在檢查固定 Rust 頻道", "Checking the configured Rust channel"],
    ["正在下载 · 优先复用旧包，失败一次全量回退", "正在下載 · 優先重用舊包，失敗時一次完整下載", "Downloading · Reusing existing blocks; one full-download fallback"],
    ["正在解包并验证 OCR · 不连接游戏", "正在解壓並驗證 OCR · 不連接遊戲", "Unpacking and checking OCR · No game connection"],
    ["正在切换版本指针 · 保留上一版本", "正在切換版本指標 · 保留上一版本", "Activating the update · Keeping the previous version"],
    ["私有频道令牌已删除。公开频道无需令牌。", "私人頻道權杖已刪除。公開頻道不需要權杖。", "Private-channel token removed. Public channels do not require a token."],
    ["已切回上一版，下次启动生效。", "已切回上一版，下次啟動生效。", "Previous version restored. It will be used on the next launch."],
    ["检查 Rust 开发频道更新？公开频道无需令牌。下载和安装均由你确认；不会操作游戏或 Electron 稳定版。", "檢查 Rust 開發頻道更新？公開頻道不需要權杖。下載及安裝均需確認；不會操作遊戲或 Electron 穩定版。", "Check for Rust preview updates? Public channels need no token. You confirm downloads and installation. The game and stable Electron app are unaffected."],
    ["当前已是同版/更新版本，或此版本曾启动失败。不会重复安装。", "目前已是相同或較新版本，或此版本曾啟動失敗。不會重複安裝。", "Already on this or a newer version, or this update previously failed to start. Nothing will be reinstalled."],
    ["发现 Rust 版本 {}，完整包约 {} MB。下载并校验？有可用基线时优先差分，失败最多回退一次全量。", "發現 Rust 版本 {}，完整包約 {} MB。下載並驗證？優先差分，失敗時最多一次完整下載。", "Rust version {} is available (full package: {} MB). Download and verify? Existing blocks are reused where possible, with at most one full-download fallback."],
    ["下载与离线 OCR 自检通过（{}，下载 {} MB）。是否安装到独立 Rust 目录？保留上一版，下一次启动前再次自检，失败自动回退。", "下載及離線 OCR 自檢通過（{}，下載 {} MB）。安裝到獨立 Rust 目錄？保留上一版，啟動驗證失敗時自動回復。", "Download and offline OCR checks passed ({}, {} MB downloaded). Install to the isolated Rust directory? The previous version is retained for startup rollback."],
    ["Rust 更新已准备好。下次使用 Start-preview.cmd 启动新版；验证失败自动回到上一版。没有重启任何程序。", "Rust 更新已準備好。下次使用 Start-preview.cmd 啟動新版；驗證失敗時回復上一版。未重新啟動任何程式。", "Rust update is ready. Use Start-preview.cmd next time; failed startup validation restores the previous version. No application was restarted."],
    ["操作未完成：{}\n原有版本与 Electron 稳定版保持不变。私有频道请使用 Private-channel.cmd 管理令牌；公开仓库无需令牌。", "操作未完成：{}\n保留原有版本及 Electron 穩定版。私人頻道請使用 Private-channel.cmd 管理權杖；公開儲存庫不需要權杖。", "Operation did not complete: {}\nThe previous version and stable Electron app are retained. Use Private-channel.cmd for private-channel credentials; public repositories need no token."],
    ["仅私有频道需要：用户名可填 GitHub，密码栏填写仅此仓库 Contents 只读令牌。公开频道无需填写。", "僅私人頻道需要：使用者名稱可填 GitHub，密碼填此儲存庫 Contents 唯讀權杖。公開頻道不需要。", "Private channels only: use GitHub as the username and a token with read-only Contents access to this repository as the password. Public channels need no credentials."],
    ["符择 · 私有更新凭据", "符擇 · 私人更新認證", "FUZE · Private update credentials"],
    ["首次观察", "首次觀察", "First observed"],
    ["候选变化", "候選變化", "Candidates changed"],
    ["再次出现", "再次出現", "Reappeared"],
    [
        "当前图标词典（非历史补丁）：",
        "目前圖示詞典（非歷史版本）：",
        "Current asset catalogue (not historical patch):",
    ],
    [
        "导入此局旧观察",
        "匯入此局舊觀察",
        "Import this match's observations",
    ],
    ["返回战绩详情", "返回戰績詳情", "Back to match details"],
    [
        "查看候选观察（非领取）",
        "查看候選觀察（非領取）",
        "Candidate observations (not claims)",
    ],
    [
        "候选观察不代表实际领取；变化或再次出现不证明刷新。",
        "候選觀察不代表實際領取；變化或再次出現不證明刷新。",
        "Observations are not claims; changes or reappearance do not prove rerolls.",
    ],
    ["中断", "中斷", "Interrupted"],
    ["达到记录上限", "達到記錄上限", "Capped"],
    [
        "* 含缺失卡位；未记录的事件不补造。",
        "* 含缺失卡位；未記錄的事件不補造。",
        "* Missing slots; unrecorded events are not reconstructed.",
    ],
    ["装备", "裝備", "Items"],
    ["战绩强化", "戰績增幅", "Result augments"],
    [
        "浏览已存账号（离线）",
        "瀏覽已存帳號（離線）",
        "Saved accounts (offline)",
    ],
    ["胜利", "勝利", "Win"],
    ["失败", "失敗", "Loss"],
    ["重开", "重開", "Remake"],
    ["上一页", "上一頁", "Previous"],
    ["下一页", "下一頁", "Next"],
    ["识别观察记录", "辨識觀察記錄", "OCR observations"],
    ["账号战绩", "帳號戰績", "Account match history"],
    ["刷新最近 50 场", "重新整理最近 50 場", "Refresh latest 50"],
    ["预览旧战绩导入", "預覽舊戰績匯入", "Preview legacy import"],
    [
        "确认导入匹配记录",
        "確認匯入匹配記錄",
        "Confirm matching import",
    ],
    [
        "暂停时手动查询客户端最近 50 场，仅保留海克斯大乱斗。",
        "暫停時手動查詢用戶端最近 50 場，僅保留海克斯大亂鬥。",
        "Manual refresh while paused. Mayhem only; current client's latest 50 matches.",
    ],
    [
        "下方为查询快照；刷新确认当前账号。查询失败不删除本地缓存。",
        "下方為查詢快照；重新整理確認目前帳號。查詢失敗不刪除本機快取。",
        "Saved snapshot; refresh to verify current account. Failed queries keep disk cache.",
    ],
    [
        "匹配账号导入预览",
        "匹配帳號匯入預覽",
        "Matching import preview",
    ],
    [
        "正在核实账号与战绩…",
        "正在核實帳號與戰績…",
        "Checking account and history...",
    ],
    ["缓存快照", "快取快照", "Cached snapshot"],
    ["最近查询快照", "最近查詢快照", "Latest query snapshot"],
    [
        "暂无已验证的战绩记录。",
        "暫無已驗證的戰績記錄。",
        "No verified records to display.",
    ],
    ["软件更新", "軟體更新", "App update"],
    ["本机识别记录", "本機辨識記錄", "Local OCR observations"],
    ["本机记录", "本機記錄", "Records"],
    ["开启本地记录", "開啟本機記錄", "Enable local records"],
    [
        "停止保存新记录",
        "停止儲存新記錄",
        "Stop saving new records",
    ],
    [
        "只记录出现过的卡片，不代表已领取或胜负；暂停、断线可能拆成多个片段。",
        "只記錄出現過的卡片，不代表已領取或勝負；暫停、斷線可能拆成多個片段。",
        "Observed cards only, not claims or results. Pause/reconnect may split a segment.",
    ],
    [
        "在线战绩查询尚未接入。记录默认关闭，不含账号、玩家名称、时间或对局编号。",
        "線上戰績尚未接入。記錄預設關閉，不含帳號、玩家名稱、時間或對局編號。",
        "Online history not connected. Recording is opt-in; no accounts, names, times or game IDs.",
    ],
    [
        "暂无记录；开启后从下一次识别开始保存。",
        "尚無記錄；開啟後從下次辨識開始儲存。",
        "No observations yet; enable to record subsequent recognition.",
    ],
    ["次观察", "次觀察", "observations"],
    ["导入旧版设置", "匯入舊版設定", "Import old settings"],
    ["我方阵容", "我方陣容", "Our team"],
    ["你 · 已选择", "你 · 已選擇", "You · Selected"],
    ["队友 · 已选择", "隊友 · 已選擇", "Ally · Selected"],
    [
        "备选英雄 · 按整体胜率排序",
        "備選英雄 · 依整體勝率排序",
        "Available champions · Overall win rate",
    ],
    [
        "当前没有可选备选英雄；客户端变化后自动刷新",
        "目前沒有備選英雄；用戶端變更後自動更新",
        "No bench champions; refreshes automatically on client changes",
    ],
    [
        "英雄详情 · 点击左侧卡片查看",
        "英雄詳情 · 點擊左側卡片查看",
        "Champion details · Select a card",
    ],
    ["出装", "出裝", "Builds"],
    ["海克斯", "海克斯", "Augments"],
    ["加点", "技能點法", "Skills"],
    ["符文页", "符文頁", "Runes"],
    ["胜率", "勝率", "Win"],
    ["胜", "勝", "W"],
    ["选", "選", "P"],
    ["等待选择英雄", "等待選擇英雄", "Waiting for champion"],
    [
        "本地详情不可用；可暂停后更新数据",
        "本機詳情不可用；可暫停後更新資料",
        "Local details unavailable; pause to update data",
    ],
    [
        "正在读取本地详情…",
        "正在讀取本機詳情…",
        "Loading local details…",
    ],
    ["滚轮浏览详情", "滾輪瀏覽詳情", "Scroll for details"],
    [
        "仅供参考，请在游戏中选择",
        "僅供參考，請在遊戲中選擇",
        "Read-only; select in game",
    ],
    ["开启", "開啟", "On"],
    ["关闭", "關閉", "Off"],
    ["自动", "自動", "Auto"],
    ["点击切换", "點擊切換", "Click to change"],
    ["跟随客户端", "跟隨用戶端", "Follow client"],
    ["收起到托盘", "收起至系統匣", "Hide to tray"],
    ["退出程序", "結束程式", "Exit app"],
    [
        "私有频道（凭据管理器）",
        "私人頻道（認證管理員）",
        "Private (Credential Manager)",
    ],
    [
        "公开频道（无令牌）",
        "公開頻道（無權杖）",
        "Public (no token)",
    ],
    ["保存设置", "儲存設定", "Save settings"],
    ["返回工作台", "返回工作台", "Dashboard"],
    ["偏好设置", "偏好設定", "Settings"],
    ["符择  /  F U Z E", "符擇  /  F U Z E", "F U Z E"],
    [
        "对局工作台 · Rust 开发预览",
        "對局工作台 · Rust 開發預覽",
        "Match dashboard · Rust preview",
    ],
    [
        "偏好设置 · 暂停识别后保存",
        "偏好設定 · 暫停辨識後儲存",
        "Settings · Pause before saving",
    ],
    ["OCR 后端", "OCR 後端", "OCR backend"],
    [
        "识别与推荐数据语言",
        "介面與推薦資料語言",
        "Interface and data language",
    ],
    ["显示选人参考", "顯示選角參考", "Champion suggestions"],
    ["选人窗口置顶", "選角視窗置頂", "Keep suggestions on top"],
    ["关闭主界面", "關閉主視窗", "On window close"],
    [
        "后台下载公共图标",
        "背景下載公開圖示",
        "Download public icons",
    ],
    ["软件更新频道", "軟體更新頻道", "Update channel"],
    [
        "暂停快捷键 Ctrl+Shift+F8",
        "暫停快捷鍵 Ctrl+Shift+F8",
        "Pause hotkey Ctrl+Shift+F8",
    ],
    [
        "高级目录回退（可留空，仅自动发现失败后使用）",
        "進階目錄備援（可留空，自動探索失敗後使用）",
        "Fallback folder (optional; used if discovery fails)",
    ],
    [
        "快捷键已被占用；仍可使用按钮或托盘暂停。",
        "快捷鍵已被占用；仍可使用按鈕或系統匣暫停。",
        "Hotkey unavailable; use the button or tray to pause.",
    ],
    [
        "更改在下次开始识别时生效。",
        "變更於下次開始辨識時生效。",
        "Changes apply when recognition next starts.",
    ],
    [
        "偏好已保存；下次开始识别生效。",
        "偏好已儲存；下次開始辨識生效。",
        "Settings saved; applied on next recognition start.",
    ],
    [
        "保存未完成；仍使用原设置。",
        "儲存未完成；仍使用原設定。",
        "Save failed; previous settings remain active.",
    ],
    [
        "正在下载并校验；旧数据继续保留。",
        "正在下載並驗證；保留原有資料。",
        "Downloading and verifying; keeping existing data.",
    ],
    [
        "推荐数据已校验；下次开始识别时使用最新完整版本。",
        "推薦資料已驗證；下次辨識使用最新完整版本。",
        "Data verified; next recognition uses the newest complete version.",
    ],
    [
        "更新未完成；保留可用数据，可稍后重试。",
        "更新未完成；保留可用資料，可稍後重試。",
        "Update incomplete; existing data kept. Retry later.",
    ],
    [
        "选人参考等待客户端语言，暂不混用不同语言数据。",
        "選角參考等待用戶端語言，不混用不同語言資料。",
        "Waiting for client language; datasets are not mixed.",
    ],
    [
        "选人数据暂不可用，30 秒后重试；可暂停后更新推荐数据。",
        "選角資料暫不可用，30 秒後重試；可暫停後更新。",
        "Champion data unavailable; retrying in 30s. Pause to update.",
    ],
    [
        "已连接 · 大厅 / 匹配中",
        "已連線 · 大廳 / 配對中",
        "Connected · Lobby / matchmaking",
    ],
    [
        "已连接 · 选择英雄",
        "已連線 · 選擇英雄",
        "Connected · Champion select",
    ],
    [
        "已连接 · 正在载入",
        "已連線 · 正在載入",
        "Connected · Loading",
    ],
    ["已连接 · 对局中", "已連線 · 對局中", "Connected · In game"],
    [
        "已连接 · 结算中",
        "已連線 · 結算中",
        "Connected · Post game",
    ],
    [
        "认证失败 · 等待重新发现",
        "驗證失敗 · 等待重新探索",
        "Authentication failed · Rediscovering",
    ],
    [
        "连接异常 · 后台重试",
        "連線異常 · 背景重試",
        "Connection error · Retrying",
    ],
    [
        "已暂停连接检测",
        "已暫停連線檢測",
        "Connection checks paused",
    ],
    [
        "等待英雄联盟客户端",
        "等待英雄聯盟用戶端",
        "Waiting for League client",
    ],
    ["画面已处理", "畫面已處理", "Frame processed"],
    [
        "等待新的可用画面",
        "等待新的可用畫面",
        "Waiting for a fresh frame",
    ],
    [
        "游戏切出 / 最小化 · 已暂停截图",
        "遊戲切出 / 最小化 · 已暫停擷取",
        "Game inactive / minimized · Capture paused",
    ],
    [
        "截图异常 · 等待重新建流",
        "擷取異常 · 等待重建串流",
        "Capture error · Rebuilding stream",
    ],
    [
        "游戏分辨率变化 · 正在重建",
        "遊戲解析度變更 · 正在重建",
        "Resolution changed · Rebuilding",
    ],
    [
        "显卡忙碌 · 已跳过本帧",
        "顯示卡忙碌 · 已略過本幀",
        "GPU busy · Frame skipped",
    ],
    [
        "等待新画面 · 不重复识别",
        "等待新畫面 · 不重複辨識",
        "Waiting for a new frame · No duplicate OCR",
    ],
    [
        "等待支持的对局与游戏窗口",
        "等待支援的對局與遊戲視窗",
        "Waiting for a supported match and game window",
    ],
    [
        "仅推荐，不操作游戏。领取轮次未确认，使用整体胜率。",
        "僅推薦，不操作遊戲。領取輪次未確認，使用整體勝率。",
        "Read-only suggestions. Claim round unconfirmed; overall win rates.",
    ],
    [
        "暂未取得支持的客户端语言；自动重试，不猜测语言。",
        "尚未取得支援的用戶端語言；自動重試。",
        "Client language unavailable; retrying without guessing.",
    ],
    [
        "尚未找到匹配的游戏窗口；客户端连接不等于已进入游戏。",
        "尚未找到對應遊戲視窗；用戶端連線不代表已進入遊戲。",
        "No matching game window yet; a connected client may be in lobby.",
    ],
    [
        "OCR 已回退到 CPU，等待下一帧继续识别。",
        "OCR 已退回 CPU，等待下一幀繼續辨識。",
        "OCR fell back to CPU; waiting for the next frame.",
    ],
    [
        "截图链路暂不可用，正在退避重试；不会操作游戏。",
        "擷取暫不可用，正在延遲重試；不會操作遊戲。",
        "Capture unavailable; retrying with backoff. No game actions.",
    ],
    [
        "OCR 进程异常；正在清理并尝试恢复。",
        "OCR 程序異常；正在清理並嘗試恢復。",
        "OCR process failed; cleaning up and recovering.",
    ],
    [
        "本地推荐数据未就绪；请检查开发包是否完整。",
        "本機推薦資料未就緒；請檢查開發套件是否完整。",
        "Local data unavailable; check the preview bundle is complete.",
    ],
    [
        "模块暂不可用，正在重试；下方仅显示脱敏错误分类。",
        "模組暫不可用，正在重試；下方僅顯示去識別錯誤分類。",
        "Module unavailable; retrying. Only sanitized error codes appear below.",
    ],
    [
        "正在更新推荐数据",
        "正在更新推薦資料",
        "Updating recommendation data",
    ],
    [
        "正在退出 · 释放识别资源",
        "正在結束 · 釋放辨識資源",
        "Exiting · Releasing recognition resources",
    ],
    [
        "正在取消更新 · 等待当前请求结束",
        "正在取消更新 · 等待目前請求結束",
        "Cancelling update · Waiting for request to finish",
    ],
    [
        "正在暂停 · 清理画面与 OCR",
        "正在暫停 · 清理畫面與 OCR",
        "Pausing · Releasing frames and OCR",
    ],
    [
        "更新未完成 · 已保留本地数据",
        "更新未完成 · 已保留本機資料",
        "Update incomplete · Local data retained",
    ],
    [
        "已暂停 · 点击开始识别",
        "已暫停 · 點擊開始辨識",
        "Paused · Click Start recognition",
    ],
    [
        "需要关注 · 正在恢复",
        "需要關注 · 正在恢復",
        "Attention needed · Recovering",
    ],
    [
        "游戏未在前台 · 截图已暂停",
        "遊戲不在前景 · 擷取已暫停",
        "Game in background · Capture paused",
    ],
    ["识别运行中", "辨識執行中", "Recognition running"],
    [
        "等待就绪 · 按对局状态自动识别",
        "等待就緒 · 依對局狀態自動辨識",
        "Waiting · Recognition follows match state",
    ],
    [
        "CPU · 实际运行后端",
        "CPU · 實際執行後端",
        "CPU · Active backend",
    ],
    [
        "DirectML · 实际运行后端",
        "DirectML · 實際執行後端",
        "DirectML · Active backend",
    ],
    [
        "待初始化 · 进入对局后启动",
        "待初始化 · 進入對局後啟動",
        "Not initialized · Starts in game",
    ],
    ["客户端连接", "用戶端連線", "Client connection"],
    ["检测已暂停", "檢測已暫停", "Checks paused"],
    ["游戏画面", "遊戲畫面", "Game capture"],
    ["未截图", "尚未擷取", "No capture"],
    ["OCR 引擎", "OCR 引擎", "OCR engine"],
    ["推荐数据", "推薦資料", "Recommendation data"],
    [
        "待选择客户端语言 · 未加载",
        "等待用戶端語言 · 尚未載入",
        "Waiting for client language · Not loaded",
    ],
    [
        "尚无本次运行的识别结果；等待阶段不代表故障。",
        "本次執行尚無辨識結果；等待階段不代表故障。",
        "No recognition result this run; waiting is not an error.",
    ],
    [
        "关闭窗口后留在托盘；结束运行请点“退出”。",
        "關閉視窗後留在系統匣；結束請點「結束」。",
        "Closing hides to tray; choose Exit to stop.",
    ],
    [
        "托盘不可用：关闭窗口将退出，避免留下不可见进程。",
        "系統匣不可用：關閉視窗將結束程式。",
        "Tray unavailable: closing the window exits the app.",
    ],
    ["打开符择", "開啟符擇", "Open FUZE"],
    ["暂停识别", "暫停辨識", "Pause"],
    ["开始识别", "開始辨識", "Start"],
    ["退出", "結束", "Exit"],
    ["更新推荐数据", "更新推薦資料", "Update data"],
    ["退出并检查更新", "結束並檢查更新", "Exit and update"],
    [
        "符择 FUZE · 双击打开，右键暂停或退出",
        "符擇 FUZE · 按兩下開啟，右鍵暫停或結束",
        "FUZE · Double-click to open; right-click to pause or exit",
    ],
    [
        "符择 · 选人参考",
        "符擇 · 選角參考",
        "FUZE · Champion suggestions",
    ],
    ["等待选择", "等待選擇", "Waiting for selection"],
    ["错误分类", "錯誤分類", "Error"],
    ["连接错误分类", "連線錯誤分類", "Connection error"],
    [
        "上次识别（秒）",
        "上次辨識（秒）",
        "Last recognition (seconds)",
    ],
];
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_unique_catalog_and_thread_local_fallback() {
        let mut keys = std::collections::HashSet::new();
        for row in MESSAGES {
            assert!(keys.insert(row[0]));
            assert!(row.iter().all(|s| !s.is_empty()));
        }
        set("en-US");
        assert_eq!(text("偏好设置"), "Settings");
        assert_eq!(
            std::thread::spawn(|| text("偏好设置").to_owned())
                .join()
                .unwrap(),
            "偏好设置"
        );
        set("zh-TW");
        assert_eq!(text("偏好设置"), "偏好設定");
        assert_eq!(text("data-error-fixed"), "data-error-fixed");
        set("zh-CN");
    }
}
