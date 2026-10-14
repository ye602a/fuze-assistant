# WGC 退出竞态处理

PR #23 的 Windows CI 38620872540 曾出现：3 帧、一次尺寸重建、非黑画面均成功，但原生测试进程最终退出码 0xC0000005。验证脚本正确判失败，没有把先打印的 verified=true 当作正常退出。

[上游 WGC 示例 issue #99](https://github.com/robmikh/Win32CaptureSample/issues/99) 记录了 Close 后内部回调与 GraphicsCapture.dll 卸载竞态的相似故障。这里没有 crash dump，因此不能确定本次故障一定具有相同调用栈。

防护措施：仅从 System32 加载 GraphicsCapture.dll，并使用 Windows PIN 标志保留其代码映射到当前测试进程退出；OnceLock 保证每进程一次，不累积每次采集的库引用。仍然关闭会话、帧池、帧对象和 D3D 资源，仍然平衡 RoInitialize/RoUninitialize。不增加每帧工作，不使用“睡一会儿应该安全”的退出等待。

这是明确的进程级常驻 DLL 取舍，不是彻底修复 Windows 内部问题，也不是零内存开销。后续正式捕获应在隔离 worker 中运行，让其进程退出回收全部状态。

CI 进行五个独立自有窗口进程的取帧/重建/退出，每一次异常退出都会使检查失败，不重试到成功掩盖问题。只在 CI 创建可见测试窗口，本机不执行该测试。
