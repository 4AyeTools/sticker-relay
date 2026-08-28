import { getVersion } from '@tauri-apps/api/app';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

type UpdateStage = 'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'error';
type UpdateErrorOrigin = 'check' | 'install';

interface UpdateSummary {
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
}

interface DownloadProgress {
  downloaded: number;
  total?: number;
}

interface Props {
  onNotice(message: string): void;
  onPrepareInstall(): Promise<void>;
}

const AUTO_CHECK_INTERVAL = 12 * 60 * 60 * 1000;
const AUTO_CHECK_STORAGE_KEY = 'sticker-relay:last-app-update-check';
let automaticCheckStarted = false;

function errorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (/release json|valid release|status code 404/i.test(message)) {
    return '更新服务还没有准备好，请稍后再试。';
  }
  if (/network|fetch|connect|dns|timed? ?out/i.test(message)) {
    return '暂时无法连接更新服务，请检查网络后重试。';
  }
  return message || '检查更新失败，请稍后重试。';
}

function formatDate(value?: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium' }).format(date);
}

export default function AppUpdater({ onNotice, onPrepareInstall }: Props) {
  const updateRef = useRef<Update | null>(null);
  const checkingRef = useRef(false);
  const [currentVersion, setCurrentVersion] = useState('');
  const [stage, setStage] = useState<UpdateStage>('idle');
  const [summary, setSummary] = useState<UpdateSummary | null>(null);
  const [progress, setProgress] = useState<DownloadProgress>({ downloaded: 0 });
  const [dialogOpen, setDialogOpen] = useState(false);
  const [error, setError] = useState('');
  const [errorOrigin, setErrorOrigin] = useState<UpdateErrorOrigin>('check');

  useEffect(() => {
    void getVersion().then(setCurrentVersion).catch(() => undefined);
  }, []);

  const releaseUpdate = useCallback(async () => {
    const previous = updateRef.current;
    updateRef.current = null;
    if (previous) await previous.close().catch(() => undefined);
  }, []);

  const runCheck = useCallback(async (manual: boolean) => {
    if (checkingRef.current || stage === 'downloading' || stage === 'installing') return;
    checkingRef.current = true;
    setStage('checking');
    setError('');
    try {
      await releaseUpdate();
      const update = await check({ timeout: 20_000 });
      try {
        window.localStorage.setItem(AUTO_CHECK_STORAGE_KEY, String(Date.now()));
      } catch {
        // Update checks still work when WebView storage is unavailable.
      }
      if (!update) {
        setStage('idle');
        if (manual) onNotice(`当前已是最新版${currentVersion ? ` · v${currentVersion}` : ''}。`);
        return;
      }
      updateRef.current = update;
      setSummary({
        currentVersion: update.currentVersion,
        version: update.version,
        date: update.date,
        body: update.body,
      });
      setProgress({ downloaded: 0 });
      setStage('available');
      setDialogOpen(true);
      onNotice(`发现表情递 v${update.version}，可以在应用内更新。`);
    } catch (checkError) {
      const message = errorMessage(checkError);
      setErrorOrigin('check');
      setStage(manual ? 'error' : 'idle');
      setError(message);
      if (manual) {
        setDialogOpen(true);
        onNotice(message);
      }
    } finally {
      checkingRef.current = false;
    }
  }, [currentVersion, onNotice, releaseUpdate, stage]);

  useEffect(() => {
    if (automaticCheckStarted) return undefined;
    automaticCheckStarted = true;
    let shouldCheck = true;
    try {
      const lastCheck = Number(window.localStorage.getItem(AUTO_CHECK_STORAGE_KEY) || 0);
      shouldCheck = !Number.isFinite(lastCheck) || Date.now() - lastCheck >= AUTO_CHECK_INTERVAL;
    } catch {
      // Fall back to one check per process when WebView storage is unavailable.
    }
    if (!shouldCheck) return undefined;
    const timer = window.setTimeout(() => void runCheck(false), 2_500);
    return () => window.clearTimeout(timer);
  }, [runCheck]);

  useEffect(() => () => {
    void releaseUpdate();
  }, [releaseUpdate]);

  const installUpdate = async () => {
    const update = updateRef.current;
    if (!update) {
      await runCheck(true);
      return;
    }
    setStage('downloading');
    setError('');
    setProgress({ downloaded: 0 });
    let downloaded = 0;
    let total: number | undefined;
    try {
      await update.download((event) => {
        if (event.event === 'Started') {
          total = event.data.contentLength;
          setProgress({ downloaded: 0, total });
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          setProgress({ downloaded, total });
        } else if (event.event === 'Finished') {
          setProgress((previous) => ({ ...previous, downloaded: previous.total ?? previous.downloaded }));
        }
      });
      setStage('installing');
      onNotice('更新包验证完成，正在安全保存会话并安装…');
      await onPrepareInstall();
      await update.install();
      await relaunch();
    } catch (installError) {
      const message = errorMessage(installError);
      setErrorOrigin('install');
      setError(message);
      setStage('error');
      setDialogOpen(true);
      onNotice(`应用更新失败：${message}`);
    }
  };

  const dismiss = async () => {
    if (stage === 'downloading' || stage === 'installing') return;
    setDialogOpen(false);
    setError('');
    setErrorOrigin('check');
    setStage('idle');
    setSummary(null);
    await releaseUpdate();
  };

  const percentage = progress.total && progress.total > 0
    ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
    : null;
  const releaseDate = formatDate(summary?.date);

  return (
    <>
      <button
        className="secondary-button update-check-button"
        type="button"
        disabled={stage === 'checking' || stage === 'downloading' || stage === 'installing'}
        onClick={() => void runCheck(true)}
      >
        <span className={`update-check-dot${summary ? ' available' : ''}`} aria-hidden="true" />
        {stage === 'checking' ? '正在检查…' : `检查更新${currentVersion ? ` · v${currentVersion}` : ''}`}
      </button>

      {dialogOpen && createPortal(
        <div className="update-backdrop" role="presentation">
          <section className="update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-dialog-title">
            <div className="update-dialog-mark" aria-hidden="true">↗</div>
            <div className="update-dialog-copy">
              <span className="update-dialog-eyebrow">
                {stage === 'error' ? 'UPDATE PAUSED' : 'NEW VERSION'}
              </span>
              <h2 id="update-dialog-title">
                {stage === 'error'
                  ? '这次更新没有完成'
                  : `表情递 v${summary?.version ?? ''} 已经到站`}
              </h2>
              {stage === 'error' ? (
                <p className="update-error-message">{error}</p>
              ) : (
                <>
                  <div className="update-version-row">
                    <span>当前 v{summary?.currentVersion}</span>
                    <i aria-hidden="true">→</i>
                    <strong>新版 v{summary?.version}</strong>
                    {releaseDate && <time>{releaseDate}</time>}
                  </div>
                  <div className="update-release-notes">
                    <strong>本次更新</strong>
                    <p>{summary?.body?.trim() || '包含新的功能改进、兼容性优化和问题修复。'}</p>
                  </div>
                </>
              )}

              {(stage === 'downloading' || stage === 'installing') && (
                <div className="app-update-progress" aria-live="polite">
                  <div className="app-update-progress-meta">
                    <strong>{stage === 'installing' ? '正在安装并准备重启…' : '正在下载安装包…'}</strong>
                    <span>{stage === 'installing' ? '已验证' : percentage === null ? '连接中' : `${percentage}%`}</span>
                  </div>
                  <div className={`app-update-progress-track${percentage === null ? ' indeterminate' : ''}`}>
                    <div style={{ width: stage === 'installing' ? '100%' : percentage === null ? '36%' : `${percentage}%` }} />
                  </div>
                  <small>安装前会保存微信登录状态；本地表情和飞书组件不会被删除。</small>
                </div>
              )}

              {stage !== 'error' && stage !== 'downloading' && stage !== 'installing' && (
                <div className="update-security-note">
                  <span aria-hidden="true">✓</span>
                  更新包会先通过表情递的 Tauri 签名验证，再交给系统安装。
                </div>
              )}
            </div>

            <div className="update-dialog-actions">
              <button
                className="secondary-button"
                type="button"
                disabled={stage === 'downloading' || stage === 'installing'}
                onClick={() => void dismiss()}
              >
                {stage === 'error' ? '稍后再试' : '稍后提醒'}
              </button>
              <button
                className="primary-button update-primary-button"
                type="button"
                disabled={stage === 'downloading' || stage === 'installing'}
                onClick={() => void (stage === 'error' && errorOrigin === 'check' ? runCheck(true) : installUpdate())}
              >
                {stage === 'error'
                  ? (errorOrigin === 'install' ? '重试更新' : '重新检查')
                  : '下载安装并重启'}
              </button>
            </div>
          </section>
        </div>,
        document.body,
      )}
    </>
  );
}
