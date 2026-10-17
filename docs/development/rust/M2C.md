# M2c：一次 CPU 恢复与 Windows 子进程归属

Supervisor 统一管理 auto 的回退预算：子进程明确运行 DML，宿主处理初始化、预热、推理返回错误、崩溃和超时；只允许一次 CPU 替代。显式 cpu/dml/dml-strict 不隐藏失败。旧 worker 必须已退出并回收才能启动 CPU；无法确认停止则不启动第二个 worker。CPU 再失败后停止，不循环重启。

推理期间失败后丢弃旧帧，返回 worker-recovered-use-next-frame，让宿主按 Session 退避处理下一张新帧，不保存截图等待重放。恢复初始化有独立截止时间，当前阻塞接口要求在后台控制线程运行；UI 线程桥接仍未实现。

Windows Job Object 设置 KILL_ON_JOB_CLOSE。私有启动握手先等待父进程完成归属设置，再加载模型。若父进程在归属之前退出，管道 EOF 使 worker 结束；归属之后则由 Job 关闭终止 worker。Job 句柄不继承；不访问或纳管任何已有进程。参考 [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)。

定向探针覆盖 GPU 子进程崩溃→CPU 成功、GPU→CPU 连续崩溃后停止、初始化崩溃→CPU、显式 DML 不回退、关闭 Job 后 child 在两秒内退出。真实离线 CPU/auto 两条链路再次通过，均连续 9 帧、只启动一次、最终回收；本机 auto 实际使用 DirectML。不是父进程强杀压力测试，也不是游戏测试。

不涉及 LCU、游戏进程、截图权限、用户输入、实际安装更新。稳定入口仍未切换。
