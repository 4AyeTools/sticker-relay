import { useCallback, useEffect, useMemo, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { getCurrentWindow } from '@tauri-apps/api/window';
import appIconUrl from '../src-tauri/icon-source.svg';
import type { StickerRecord } from './shared/types';
import FeishuPanel from './components/FeishuPanel';
import StickerGrid from './components/StickerGrid';
import WechatPanel from './components/WechatPanel';

export default function App() {
  const [stickers, setStickers] = useState<StickerRecord[]>([]);
  const [notice, setNotice] = useState('表情只保存在本机；微信登录状态会加密保留，方便下次继续使用。');
  const [startupNotice, setStartupNotice] = useState<{ title: string; message: string } | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exitDialogOpen, setExitDialogOpen] = useState(false);
  const [exitAction, setExitAction] = useState<'keep' | 'logout' | null>(null);

  const refresh = useCallback(async () => {
    setStickers(await window.desktop.stickers.list());
  }, []);

  useEffect(() => {
    void refresh();
    return window.desktop.events.onStickerImported((event) => {
      if (event.warning) setNotice(`微信同步提示：${event.warning}`);
      else if (event.imported > 0) setNotice(`刚刚采集了 ${event.imported} 个表情。`);
      else if (event.unsupported > 0) setNotice(`${event.unsupported} 个表情无法从网页版下载。`);
      void refresh();
    });
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    void getVersion().then((version) => {
      const storageKey = 'sticker-relay:last-seen-version';
      const legacyStorageKey = 'xiuxiuban:last-seen-version';
      let previousVersion: string | null = null;
      try {
        previousVersion = window.localStorage.getItem(storageKey)
          ?? window.localStorage.getItem(legacyStorageKey);
        window.localStorage.setItem(storageKey, version);
      } catch {
        // The notification still works when WebView storage is unavailable.
      }
      if (disposed || previousVersion === version) return;
      setStartupNotice({
        title: previousVersion ? `已升级至 v${version}` : `安装完成 · v${version}`,
        message: previousVersion
          ? '新版本已经准备好，你的微信登录状态和本地表情都已保留。'
          : '表情递已经准备好，可以开始接收和递送表情。',
      });
    }).catch(() => {
      if (disposed) return;
      setStartupNotice({
        title: '表情递已就绪',
        message: '安装或升级已经完成，可以开始接收和迁移表情。',
      });
    });
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void appWindow.onCloseRequested((event) => {
      event.preventDefault();
      setExitDialogOpen(true);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!exitDialogOpen || exitAction) return undefined;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setExitDialogOpen(false);
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [exitAction, exitDialogOpen]);

  const counts = useMemo(() => ({
    total: stickers.length,
    pending: stickers.filter((item) => item.feishuState !== 'sent').length,
    sent: stickers.filter((item) => item.feishuState === 'sent').length,
  }), [stickers]);

  const sentRatio = counts.total > 0 ? Math.round((counts.sent / counts.total) * 100) : 0;

  const exportZip = async () => {
    setExporting(true);
    try {
      const result = await window.desktop.stickers.exportZip();
      if (!result.canceled) setNotice(`已导出 ${result.count} 个表情到 ${result.path}`);
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setExporting(false);
    }
  };

  const minimizeWindow = () => void getCurrentWindow().minimize();
  const toggleMaximizeWindow = () => void getCurrentWindow().toggleMaximize();
  const closeWindow = () => setExitDialogOpen(true);

  const finishExit = async (action: 'keep' | 'logout') => {
    setExitAction(action);
    try {
      if (action === 'keep') await window.desktop.wechat.prepareExit();
      else await window.desktop.wechat.logout();
      await getCurrentWindow().destroy();
    } catch (error) {
      setExitAction(null);
      setNotice(`关闭前处理失败：${error instanceof Error ? error.message : String(error)}`);
    }
  };

  return (
    <div className="app-shell">
      <div className="aurora-orb aurora-orb-one" />
      <div className="aurora-orb aurora-orb-two" />
      <div className="aurora-orb aurora-orb-three" />

      <div className="window-titlebar" data-tauri-drag-region onDoubleClick={toggleMaximizeWindow}>
        <div className="window-title" data-tauri-drag-region>
          <img className="window-title-icon" src={appIconUrl} alt="" aria-hidden="true" />
          <span data-tauri-drag-region>表情递</span>
        </div>
        <div className="window-controls">
          <button type="button" aria-label="最小化" title="最小化" onClick={minimizeWindow}>
            <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 6.5h8" /></svg>
          </button>
          <button type="button" aria-label="最大化或还原" title="最大化或还原" onClick={toggleMaximizeWindow}>
            <svg viewBox="0 0 12 12" aria-hidden="true"><rect x="2.5" y="2.5" width="7" height="7" rx="1" /></svg>
          </button>
          <button className="window-close" type="button" aria-label="关闭" title="关闭" onClick={closeWindow}>
            <svg viewBox="0 0 12 12" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
          </button>
        </div>
      </div>

      {startupNotice && (
        <aside className="startup-toast" role="status" aria-live="polite">
          <span className="startup-toast-mark" aria-hidden="true">✓</span>
          <div>
            <strong>{startupNotice.title}</strong>
            <span>{startupNotice.message}</span>
          </div>
          <button type="button" aria-label="关闭安装完成提示" onClick={() => setStartupNotice(null)}>×</button>
        </aside>
      )}

      <header className="hero">
        <div className="brand-lockup">
          <img className="brand-mark" src={appIconUrl} alt="表情递图标" />
          <div>
            <div className="eyebrow">StickerRelay</div>
            <h1>表情递</h1>
            <p>把收藏的表情，递到下一个平台。</p>
          </div>
        </div>
        <div className="hero-actions">
          <span className="privacy-chip"><i />仅保存在本机</span>
          <button className="secondary-button export-button" disabled={exporting || stickers.length === 0} onClick={exportZip}>
            {exporting ? '正在打包…' : '打包导出'}
          </button>
        </div>
      </header>

      <div className="notice" role="status" aria-live="polite">
        <span className="notice-spark" />
        <div><strong>最新动态</strong><span>{notice}</span></div>
      </div>

      <section className="stats-grid">
        <div className="stat-card stat-local">
          <span className="stat-icon">库</span>
          <div><span>表情仓库</span><strong>{counts.total}</strong><small>自动去重，稳稳收好</small></div>
        </div>
        <div className="stat-card stat-pending">
          <span className="stat-icon">待</span>
          <div><span>等待递送</span><strong>{counts.pending}</strong><small>一键递到飞书</small></div>
        </div>
        <div className="stat-card stat-sent">
          <span className="stat-icon">✓</span>
          <div><span>已经搬好</span><strong>{counts.sent}</strong><small>{sentRatio}% 已送达</small></div>
        </div>
      </section>

      <main className="workspace">
        <div className="left-column">
          <WechatPanel stickerCount={stickers.length} onNotice={setNotice} />
          <FeishuPanel
            stickerCount={stickers.length}
            pendingCount={counts.pending}
            onNotice={setNotice}
            onUpdated={refresh}
          />
        </div>
        <StickerGrid stickers={stickers} onUpdated={refresh} onNotice={setNotice} />
      </main>

      {exitDialogOpen && (
        <div className="exit-backdrop" role="presentation">
          <section className="exit-dialog" role="dialog" aria-modal="true" aria-labelledby="exit-dialog-title">
            <div className="exit-dialog-mark" aria-hidden="true">递</div>
            <div className="exit-dialog-copy">
              <span className="exit-dialog-eyebrow">下次接着搬</span>
              <h2 id="exit-dialog-title">表情还在路上，要先关掉吗？</h2>
              <p>保留微信登录后，下次打开会自动继续监听文件传输助手，不用重新扫码。</p>
              <div className="exit-security-note">
                <span aria-hidden="true">✓</span>
                登录状态使用系统账户级安全存储（Windows DPAPI / macOS Keychain），仅保存在本机。
              </div>
            </div>
            <div className="exit-dialog-actions">
              <button
                className="secondary-button danger-button"
                type="button"
                disabled={exitAction !== null}
                onClick={() => void finishExit('logout')}
              >
                {exitAction === 'logout' ? '正在退出微信…' : '退出微信并关闭'}
              </button>
              <button
                className="secondary-button"
                type="button"
                disabled={exitAction !== null}
                onClick={() => setExitDialogOpen(false)}
              >
                继续使用
              </button>
              <button
                className="primary-button exit-primary-button"
                type="button"
                disabled={exitAction !== null}
                onClick={() => void finishExit('keep')}
              >
                {exitAction === 'keep' ? '正在安全保存…' : '保留登录并关闭'}
              </button>
            </div>
            <small className="exit-logout-hint">选择“退出微信并关闭”后，下次需要重新扫码。</small>
          </section>
        </div>
      )}
    </div>
  );
}
