// 只用于在普通浏览器里预览界面：模拟 Tauri 的接口，数据来自 data.js（由 isobox status --json 导出）。
// 不会打包进程序。
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      await new Promise((r) => setTimeout(r, 150));
      switch (cmd) {
        case 'overview': return JSON.parse(JSON.stringify(window.MOCK_OVERVIEW));
        case 'app_info': return { version: '0.1.0', config: window.MOCK_OVERVIEW.config_path, install_dir: window.MOCK_OVERVIEW.install_dir, uninstaller: true, license: 'C:\\Program Files\\IsoBox\\licenses\\LICENSE.txt', notices: 'C:\\Program Files\\IsoBox\\licenses\\THIRD-PARTY-NOTICES.txt' };
        case 'run_uninstaller': return true;
        case 'preview_installer': await new Promise((r) => setTimeout(r, 800)); return window.MOCK_PREVIEW;
        case 'launch_app': return `已以 isobox 身份启动 ${args.name}（模拟）`;
        case 'admin':
          await new Promise((r) => setTimeout(r, 1200));
          return { code: 0, cancelled: false, output: `（模拟）isobox-setup ${args.args.join(' ')}\n完成。` };
        case 'refresh_tray': case 'set_app_startup': case 'set_tray_autostart': case 'open_path': return null;
        default: throw new Error('未模拟的命令 ' + cmd);
      }
    },
  },
  dialog: {
    open: async (opts) => (opts.directory ? 'D:\\Data' : 'D:\\Downloads\\WeChatWin_4.1.15.exe'),
  },
};
