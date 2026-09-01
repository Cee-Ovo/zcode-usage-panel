ZCode Usage Panel — Portable 版本说明
=====================================

这是免安装的 portable 版本:

1. 解压到任意目录(支持中文/Unicode 路径)。
2. 双击 ZCode-Usage-Panel.exe 运行。
3. 首次运行会自动检测 %USERPROFILE%\.zcode(或 ZCODE_HOME)。

与安装版的区别:
- 解压运行不写注册表、不创建开始菜单项、默认不注册开机启动
  (设置中的"开机启动"并未针对 portable 停用,开启后启动项会指向
   解压目录内的本程序)。
- 配置与缓存写入 %APPDATA%\com.zcode.usagepanel\,删除该目录即可完全卸载。
- 卸载/删除时不会影响你主动导出的任何数据。

单实例说明:
- 第二次启动不会出现第二个监控进程,只会唤出已有窗口。

License: MIT(本程序)/ MIT(open-glass-ui)/ 见 THIRD-PARTY-NOTICES.md
