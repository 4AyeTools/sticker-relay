import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { confirm, open, save } from '@tauri-apps/plugin-dialog';
import type {
  DesktopApi,
  ExportResult,
  FeishuCliProgress,
  FeishuLoginAdvance,
  FeishuLoginSession,
  FeishuSendProgress,
  StickerImportEvent,
  StickerDeleteResult,
  StickerLibraryChangeResult,
  StickerLibraryLocation,
} from './shared/types';

function subscribe<T>(eventName: string, callback: (payload: T) => void): () => void {
  let disposed = false;
  let unlisten: UnlistenFn | undefined;
  void listen<T>(eventName, (event) => callback(event.payload)).then((cleanup) => {
    if (disposed) cleanup();
    else unlisten = cleanup;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export function installDesktopBridge(): void {
  const api: DesktopApi = {
    wechat: {
      requestQr: () => invoke('wechat_request_qr'),
      poll: (uuid, tip) => invoke('wechat_poll', { uuid, tip }),
      status: () => invoke('wechat_status'),
      prepareExit: () => invoke('wechat_prepare_exit'),
      logout: () => invoke('wechat_logout'),
    },
    stickers: {
      list: () => invoke('stickers_list'),
      dataUrl: (id) => invoke('stickers_data_url', { id }),
      async exportZip(): Promise<ExportResult> {
        const destination = await save({
          title: '导出本地表情库',
          defaultPath: `微信表情-${new Date().toISOString().slice(0, 10)}.zip`,
          filters: [{ name: 'ZIP 压缩包', extensions: ['zip'] }],
        });
        if (!destination) return { canceled: true };
        return invoke('stickers_export_zip', { destination });
      },
      location: () => invoke('stickers_location'),
      async chooseLocation(): Promise<StickerLibraryChangeResult> {
        const current = await invoke<StickerLibraryLocation>('stickers_location');
        const destination = await open({
          title: '选择新的表情库文件夹',
          defaultPath: current.path,
          directory: true,
          multiple: false,
          canCreateDirectories: true,
        });
        if (!destination || Array.isArray(destination)) {
          return { ...current, canceled: true, migratedCount: 0 };
        }
        if (destination.toLowerCase() !== current.path.toLowerCase()) {
          const accepted = await confirm(
            `将本地表情迁移到：\n${destination}\n\n复制和校验成功后会切换到新目录。`,
            { title: '迁移本地表情库', kind: 'warning' },
          );
          if (!accepted) return { ...current, canceled: true, migratedCount: 0 };
        }
        return invoke('stickers_choose_location', { destination });
      },
      async openLocation(): Promise<void> {
        await invoke('stickers_open_location');
      },
      async remove(ids: string[], clearAll = false): Promise<StickerDeleteResult> {
        if (ids.length === 0) return { canceled: true, removed: 0 };
        const action = clearAll
          ? `清空本地表情库中的 ${ids.length} 个表情`
          : ids.length === 1
            ? '删除这个本地表情'
            : `删除选中的 ${ids.length} 个本地表情`;
        const accepted = await confirm(
          `确定要${action}吗？\n\n只会删除本地文件和迁移记录，不会删除微信收藏，也不会撤回已经发送到飞书的消息。此操作不可撤销。`,
          { title: clearAll ? '清空本地表情库' : '删除本地表情', kind: 'warning' },
        );
        if (!accepted) return { canceled: true, removed: 0 };
        const removed = await invoke<number>('stickers_delete', { ids });
        return { canceled: false, removed };
      },
    },
    feishu: {
      status: () => invoke('feishu_status'),
      checkUpdate: () => invoke('feishu_check_update'),
      installCli: () => invoke('feishu_cli_install'),
      self: () => invoke('feishu_self'),
      startLogin: () => invoke<FeishuLoginSession>('feishu_login_start'),
      openLoginPage: () => invoke('feishu_login_open'),
      finishLogin: () => invoke<FeishuLoginAdvance>('feishu_login_finish'),
      cancelLogin: () => invoke('feishu_login_cancel'),
      send: (request) => invoke('feishu_send', { request }),
    },
    events: {
      onStickerImported: (callback) => subscribe<StickerImportEvent>('stickers-imported', callback),
      onFeishuProgress: (callback) => subscribe<FeishuSendProgress>('feishu-progress', callback),
      onFeishuCliProgress: (callback) => subscribe<FeishuCliProgress>('feishu-cli-progress', callback),
    },
  };

  window.desktop = api;
}
