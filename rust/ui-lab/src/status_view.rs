//! Player-facing state comes from verified host facts, not a process or a remembered card.
use fuze_lcu::{reconnect::State, Error};
use fuze_runtime::session::Phase;
use fuze_worker_host::desktop::Status;
pub struct View {
    pub health: i32,
    pub title: &'static str,
    pub hint: &'static str,
    pub connection: &'static str,
    pub phase: &'static str,
    pub connection_tone: i32,
    pub capture: &'static str,
    pub capture_hint: &'static str,
    pub capture_tone: i32,
    pub ocr: &'static str,
    pub ocr_hint: &'static str,
    pub ocr_tone: i32,
}
pub fn present(s: &Status, offline: bool) -> View {
    let phase = match s.connection {
        Some(State::Connected(p)) => p,
        _ => Phase::Disconnected,
    };
    let running = s.mode == 1;
    let active = running && phase == Phase::InProgress;
    let waiting_window = s.pipeline.error == Some("game-window-waiting");
    let failed = s.pipeline.error.is_some() && !waiting_window && running;
    let partial = s.pipeline.recent.as_ref().is_some_and(|slots| {
        slots
            .iter()
            .any(|v| matches!(v, fuze_contract::Slot::Unreadable))
    });
    let fresh = s.pipeline.frame_age.is_some_and(|age| age <= 5);
    let mut v = View {
        health: 0,
        title: "等待英雄联盟客户端",
        hint: "登录后自动连接，通信成功后显示游戏阶段。",
        connection: "尚未验证",
        phase: "等待客户端",
        connection_tone: 0,
        capture: "正常待命",
        capture_hint: "进入对局后开始截图",
        capture_tone: 0,
        ocr: "正常待命",
        ocr_hint: "等待允许识别的游戏阶段",
        ocr_tone: 0,
    };
    if offline {
        v.connection = "隔离状态样例";
        v.phase = "未访问客户端";
        v.capture_hint = "隔离会话不截图";
        v.ocr_hint = "隔离会话不启动 OCR";
    } else {
        match s.connection {
            Some(State::Connected(p)) => {
                v.connection = "通信已验证";
                v.connection_tone = 1;
                v.phase = match p {
                    Phase::ClientReady => "大厅 / 匹配",
                    Phase::ChampSelect => "正在选择英雄",
                    Phase::Loading => "正在加载对局",
                    Phase::InProgress => "对局进行中",
                    Phase::PostGame => "对局已结束",
                    Phase::Disconnected => "阶段尚未确认",
                };
            }
            Some(State::Retry(error)) => {
                let (title, hint, code) = connection_error(error);
                v.connection = title;
                v.connection_tone = 3;
                v.phase = code;
                v.health = 3;
                v.title = title;
                v.hint = hint;
            }
            Some(State::Paused) => {
                v.connection = "监听已暂停";
                v.phase = "继续识别后重新验证";
                v.connection_tone = 2;
            }
            _ => {}
        }
    }
    if !running {
        v.health = 2;
        v.title = "识别已暂停";
        v.hint = "继续识别后恢复客户端监听与对局识别。";
        v.capture = "已暂停";
        v.capture_tone = 2;
        v.ocr = "已暂停";
        v.ocr_tone = 2;
        return v;
    }
    if offline {
        v.title = "隔离宿主 · 等待对局样例";
        v.hint = "使用固定样例，不访问游戏或个人数据。";
        return v;
    }
    match phase {
        Phase::ClientReady => {
            v.title = "已连接 · 等待对局";
            v.hint = "客户端通信正常，进入对局后才会开始截图识别。";
        }
        Phase::ChampSelect => {
            v.title = "已连接 · 正在选择英雄";
            v.hint = "英雄详情跟随阵容变化；此阶段不截图识别海克斯。";
        }
        Phase::Loading => {
            v.title = "正在加载 · 准备识别";
            v.hint = "加载阶段预热识别引擎，进入对局后开始截图。";
            v.ocr = "正在准备";
        }
        Phase::PostGame => {
            v.title = "对局已结束 · 等待下一局";
            v.hint = "本局截图与识别已停止。";
            v.capture_hint = "赛后不继续截图";
        }
        Phase::InProgress => {
            v.title = "对局中 · 等待海克斯";
            v.hint = "按等级与选择画面调度识别，未出现海克斯时待命正常。";
        }
        _ => {}
    }
    if matches!(phase, Phase::Loading | Phase::InProgress) {
        if let Some(backend) = s.pipeline.backend {
            v.ocr = if matches!(backend, fuze_contract::wire::ActiveBackend::Cpu) {
                "CPU 已就绪"
            } else {
                "DirectML 已就绪"
            };
            v.ocr_hint = "识别引擎已初始化";
            v.ocr_tone = 1;
        }
    }
    if active {
        v.capture = if fresh {
            "画面已获取"
        } else {
            "等待新画面"
        };
        v.capture_hint = if fresh {
            "最近 5 秒内获取到画面"
        } else {
            "等级门禁或游戏窗口可能正在等待"
        };
        v.capture_tone = if fresh { 1 } else { 0 };
        if s.pipeline.recent.is_some() {
            v.health = if partial { 2 } else { 1 };
            v.title = if partial {
                "部分卡位暂不可读"
            } else {
                "海克斯推荐已更新"
            };
            v.hint = if partial {
                "可能遮挡或正在刷新；保留实际卡位与上次可用结果。"
            } else {
                "保持实际卡位顺序，推荐继续使用整体胜率。"
            };
        }
    }
    if active && waiting_window {
        v.health = 2;
        v.title = "等待游戏画面";
        v.hint = "已连接客户端，正在等待对应游戏窗口。";
        v.capture = "等待游戏窗口";
        v.capture_hint = "客户端连接成功不代表画面已获取";
        v.capture_tone = 2;
    }
    if failed {
        v.health = 3;
        v.title = "识别异常 · 等待恢复";
        v.hint = "识别链路暂不可用，原有设置与数据继续保留。";
        if s.pipeline
            .error
            .is_some_and(|e| e.starts_with("capture") || e.starts_with("game-window"))
        {
            v.capture = "画面获取异常";
            v.capture_hint = "截图服务正在重试";
            v.capture_tone = 3;
        } else {
            v.ocr = "识别链路异常";
            v.ocr_hint = "正在重试，已有结果保留";
            v.ocr_tone = 3;
        }
    }
    v
}
fn connection_error(error: Error) -> (&'static str, &'static str, &'static str) {
    match error {
        Error::Discovery => (
            "无法取得客户端连接信息",
            "自动发现正在重试；可在偏好设置检查游戏目录回退。",
            "LCU-DISCOVERY",
        ),
        Error::AuthInput => (
            "客户端连接信息无效",
            "正在重新发现连接信息；无需填写或发送临时密码。",
            "LCU-CREDENTIALS",
        ),
        Error::AmbiguousClient => (
            "检测到多个客户端",
            "无法可靠确认当前客户端，暂不连接；设置和本地记录仍可查看。",
            "LCU-MULTIPLE",
        ),
        Error::Unauthorized => (
            "客户端认证失败",
            "客户端拒绝了临时凭据，正在重新发现并重试。",
            "LCU-AUTH",
        ),
        Error::NotFound => (
            "客户端接口暂不可用",
            "请求的接口返回 404，尚未验证当前游戏阶段；正在重试。",
            "LCU-NOT-FOUND",
        ),
        Error::HttpStatus => (
            "客户端响应异常",
            "客户端返回非成功状态，正在重试；未判定连接成功。",
            "LCU-HTTP",
        ),
        Error::InvalidPhase => (
            "游戏阶段尚未确认",
            "返回内容未通过阶段校验，正在重试；设置仍可使用。",
            "LCU-PHASE",
        ),
        Error::ResponseLimit => (
            "客户端响应超出限制",
            "未读取超限响应，正在重试；设置仍可使用。",
            "LCU-LIMIT",
        ),
        Error::Transport => (
            "客户端通信失败",
            "本机连接或安全握手未完成，正在重试；设置仍可使用。",
            "LCU-TRANSPORT",
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_failures_have_safe_distinct_visible_reasons() {
        let mut s = Status::default();
        s.mode = 1;
        let mut codes = std::collections::HashSet::new();
        for error in [
            Error::Discovery,
            Error::AuthInput,
            Error::AmbiguousClient,
            Error::Unauthorized,
            Error::NotFound,
            Error::HttpStatus,
            Error::InvalidPhase,
            Error::ResponseLimit,
            Error::Transport,
        ] {
            s.connection = Some(State::Retry(error));
            let v = present(&s, false);
            assert_ne!(v.title, "客户端连接异常");
            assert_eq!(v.title, v.connection);
            assert!(codes.insert(v.phase));
            assert_eq!(v.health, 3);
        }
        s.connection = Some(State::Waiting);
        assert_eq!(present(&s, false).title, "等待英雄联盟客户端");
    }
    #[test]
    fn authentication_and_phase_override_stale_cards() {
        let mut s = Status::default();
        s.mode = 1;
        s.pipeline.recent = Some(std::sync::Arc::new([
            fuze_contract::Slot::Unreadable,
            fuze_contract::Slot::Unreadable,
            fuze_contract::Slot::Unreadable,
        ]));
        s.connection = Some(State::Retry(Error::Unauthorized));
        let v = present(&s, false);
        assert_eq!(v.health, 3);
        assert_ne!(v.connection, "通信已验证");
        s.connection = Some(State::Connected(Phase::ClientReady));
        let v = present(&s, false);
        assert_eq!(v.health, 0);
        assert_eq!(v.title, "已连接 · 等待对局");
        s.connection = Some(State::Connected(Phase::InProgress));
        assert_eq!(present(&s, false).health, 2);
        s.pipeline.error = Some("game-window-waiting");
        assert_eq!(present(&s, false).capture, "等待游戏窗口");
        assert_eq!(present(&s, false).health, 2);
        s.pipeline.error = Some("capture-timeout");
        assert_eq!(present(&s, false).capture_tone, 3);
        assert_eq!(present(&s, true).connection, "隔离状态样例");
    }
}
