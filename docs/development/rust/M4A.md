# M4a：只读 LCU 通信边界

fuze-lcu 提供进程参数/lockfile 解析、固定 loopback HTTPS 传输和游戏阶段事件解析。模块没有自动运行入口；本阶段没有枚举客户端进程、读 lockfile、连接实际 LCU、读账号或操作游戏。

凭据只允许进程发现适配层传入，参数解析要求 LeagueClientUx.exe，拒绝重复端口/认证参数；lockfile 必须是 LeagueClient、https 且 PID 与观测值一致。端口不能为零，凭据有长度/字符限制。Credentials 不可 Serialize/Clone，Debug 固定脱敏；自己的密码、Basic 中间字节和 UTF-16 头缓冲在释放时覆盖。操作系统/调用者的副本不在此保证范围。

Windows WinHTTP 客户端只能 GET 固定 127.0.0.1 的两个端点：gameflow-phase 和 current-summoner 状态。没有任意 URL、代理、重定向、Cookie、系统自动认证或写接口。仅固定 loopback 请求允许 LCU 自签证书 CA/CN 例外，不放宽全局 TLS、证书期限或其它远端请求。参考 [WinHTTP 选项](https://learn.microsoft.com/en-us/windows/win32/winhttp/option-flags)。

只把 HTTP 200 加已知游戏阶段 JSON 视为有效阶段；401/403、404、其它状态、未知阶段、超长正文分别失败。账号接口不调用 ReadData，不解析/保存其正文。事件解析仅接受已认证连接交来的 gameflow WAMP 更新，不把解析成功当成 WebSocket 已认证。

解析测试与 Windows release 编译通过，没有真实网络测试。每操作 WinHTTP 超时为 500ms，阶段正文限 128 字节并附读取时长检查；这不等于整个 WinHTTP 调用有硬截止时间，文档见 [WinHttpSetTimeouts](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpsettimeouts)。未来宿主必须在独立 IO worker 接入硬取消，不能在 UI/OCR 线程调用。

仍缺少：Windows 进程优先发现适配器、已退出/复用 PID 的生命周期判定、真实国服证书/端口验证、WebSocket 连接与重连、请求监督、选人数据、语言查询。不能把这份已编译模块当成国服自动发现已修好。
