import { useCallback, useEffect, useMemo, useState } from 'react';
import type {
  FeishuCliProgress,
  FeishuCliStatus,
  FeishuDestination,
  FeishuLoginSession,
  FeishuSendProgress,
} from '../shared/types';

interface Props {
  stickerCount: number;
  pendingCount: number;
  onNotice: (message: string) => void;
  onUpdated: () => Promise<void>;
}

interface ConnectionFeedback {
  tone: 'checking' | 'success' | 'warning' | 'error';
  title: string;
  message: string;
  checkedAt?: string;
}

function currentTime(): string {
  return new Intl.DateTimeFormat('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(new Date());
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export default function FeishuPanel({ stickerCount, pendingCount, onNotice, onUpdated }: Props) {
  const [status, setStatus] = useState<FeishuCliStatus | null>(null);
  const [checking, setChecking] = useState(true);
  const [installingCli, setInstallingCli] = useState(false);
  const [cliProgress, setCliProgress] = useState<FeishuCliProgress | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [finishingLogin, setFinishingLogin] = useState(false);
  const [loginSession, setLoginSession] = useState<FeishuLoginSession | null>(null);
  const [sending, setSending] = useState(false);
  const [progress, setProgress] = useState<FeishuSendProgress | null>(null);
  const [destinationKind, setDestinationKind] = useState<'self' | 'user' | 'chat'>('self');
  const [destinationId, setDestinationId] = useState('');
  const [onlyPending, setOnlyPending] = useState(true);
  const [connectionFeedback, setConnectionFeedback] = useState<ConnectionFeedback>({
    tone: 'checking',
    title: '正在检查飞书连接',
    message: '正在验证本地 CLI 组件和登录身份。',
  });

  const checkStatus = useCallback(async (announce = false) => {
    setChecking(true);
    setConnectionFeedback({
      tone: 'checking',
      title: '正在检查飞书连接',
      message: '正在验证本地 CLI 组件和登录身份。',
    });
    try {
      const nextStatus = await window.desktop.feishu.checkUpdate();
      const checkedAt = currentTime();
      setStatus(nextStatus);

      const feedback: ConnectionFeedback = !nextStatus.installed
        ? {
            tone: 'warning',
            title: '需要下载飞书官方组件',
            message: nextStatus.detail || '首次使用需下载飞书官方 CLI，程序会自动校验文件完整性。',
            checkedAt,
          }
        : nextStatus.authenticated
          ? {
              tone: 'success',
              title: '连接检查通过',
              message: `${nextStatus.detail || '飞书身份有效'}${nextStatus.version ? ` · ${nextStatus.version}` : ''}${nextStatus.updateAvailable ? ` · 可更新至 ${nextStatus.latestVersion}` : ''}`,
              checkedAt,
            }
          : {
              tone: 'warning',
              title: '飞书尚未完成授权',
              message: nextStatus.detail || '组件运行正常，请连接飞书账号。',
              checkedAt,
            };

      setConnectionFeedback(feedback);
      if (announce) onNotice(`${feedback.title}：${feedback.message}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setConnectionFeedback({
        tone: 'error',
        title: '连接检查失败',
        message,
        checkedAt: currentTime(),
      });
      onNotice(`连接检查失败：${message}`);
    } finally {
      setChecking(false);
    }
  }, [onNotice]);

  useEffect(() => {
    void checkStatus(false);
    const stopSendProgress = window.desktop.events.onFeishuProgress((event) => {
      setProgress(event);
      if (event.done) {
        setSending(false);
        onNotice(`飞书发送完成：成功 ${event.sent}，失败 ${event.failed}。`);
        void onUpdated();
      }
    });
    const stopCliProgress = window.desktop.events.onFeishuCliProgress((event) => {
      setCliProgress(event);
    });
    return () => {
      stopSendProgress();
      stopCliProgress();
    };
  }, [checkStatus, onNotice, onUpdated]);

  const targetCount = onlyPending ? pendingCount : stickerCount;
  const cliProgressPercent = cliProgress?.total
    ? Math.min(100, (cliProgress.downloaded / cliProgress.total) * 100)
    : installingCli
      ? 18
      : 0;
  const cliAttemptLabel = cliProgress?.attempt && cliProgress.maxAttempts
    ? `第 ${cliProgress.attempt}/${cliProgress.maxAttempts} 次${cliProgress.source ? ` · ${cliProgress.source}` : ''}`
    : cliProgress?.source;
  const destination = useMemo<FeishuDestination>(() => {
    if (destinationKind === 'self') return { kind: 'self' };
    return { kind: destinationKind, id: destinationId.trim() };
  }, [destinationKind, destinationId]);

  const send = async () => {
    if (destinationKind !== 'self' && !destinationId.trim()) {
      onNotice('请输入飞书 open_id 或 chat_id。');
      return;
    }
    const targetLabel = destinationKind === 'self' ? '当前登录的飞书账号' : destinationId.trim();
    if (!window.confirm(`即将以当前飞书用户身份向 ${targetLabel} 发送 ${targetCount} 张图片，是否继续？`)) return;
    setSending(true);
    setProgress({ current: 0, total: targetCount, sent: 0, failed: 0, done: false });
    try {
      await window.desktop.feishu.send({ destination, onlyPending });
    } catch (error) {
      setSending(false);
      onNotice(error instanceof Error ? error.message : String(error));
    }
  };

  const installCli = async () => {
    setInstallingCli(true);
    setCliProgress({
      stage: 'resolving',
      downloaded: 0,
      message: '正在读取飞书官方版本与校验信息…',
      done: false,
    });
    try {
      const nextStatus = await window.desktop.feishu.installCli();
      setStatus(nextStatus);
      onNotice(`飞书官方组件 ${nextStatus.version || ''} 已安装并通过 SHA-256 校验。`);
      await checkStatus(false);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCliProgress((current) => ({
        stage: 'error',
        downloaded: current?.downloaded || 0,
        total: current?.total,
        message,
        done: true,
        attempt: current?.attempt,
        maxAttempts: current?.maxAttempts,
        source: current?.source,
      }));
      onNotice(`飞书组件安装失败：${message}`);
    } finally {
      setInstallingCli(false);
    }
  };

  const startLogin = async () => {
    setConnecting(true);
    try {
      const session = await window.desktop.feishu.startLogin();
      setLoginSession(session);
      onNotice('飞书授权页已打开，请在浏览器中确认授权。');
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
      await checkStatus(false);
    } finally {
      setConnecting(false);
    }
  };

  const finishLogin = async () => {
    setFinishingLogin(true);
    try {
      const nextStatus = await window.desktop.feishu.finishLogin();
      setStatus(nextStatus);
      setLoginSession(null);
      onNotice('飞书已连接，可以把表情递给自己了。');
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setFinishingLogin(false);
    }
  };

  const cancelLogin = async () => {
    try {
      await window.desktop.feishu.cancelLogin();
      setLoginSession(null);
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
    }
  };

  const openLoginPage = async () => {
    try {
      await window.desktop.feishu.openLoginPage();
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <section className="panel">
      <div className="panel-heading">
        <div>
          <span className="step">02</span>
          <h2>递到飞书</h2>
        </div>
        <span className={`status-dot ${status?.authenticated ? 'online' : ''}`}>
          {!status ? '正在检查' : status.installed ? (status.authenticated ? '飞书已连接' : '等待连接') : '需要组件'}
        </span>
      </div>

      <p className="panel-description">
        点击按钮后会自动打开飞书授权页。连接成功后，程序使用飞书官方组件上传图片并发给自己。
        <span className="add-sticker-text">发送完成后，在飞书中打开图片，点击「＋ 添加表情」即可收藏。</span>
      </p>

      <div className={`connection-feedback ${connectionFeedback.tone}`} role="status" aria-live="polite">
        <span className="feedback-orb" aria-hidden="true" />
        <div>
          <strong>{connectionFeedback.title}</strong>
          <span>{connectionFeedback.message}</span>
        </div>
        {connectionFeedback.checkedAt && <time>{connectionFeedback.checkedAt}</time>}
      </div>

      {status?.installed && status.updateAvailable && (
        <div className={`component-update-card ${cliProgress?.stage === 'error' ? 'has-error' : ''}`}>
          <div className="component-update-header">
            <div className="component-update-title">
              <strong>飞书官方组件可更新</strong>
              <span>{status.version || '当前版本'} → {status.latestVersion}</span>
            </div>
            <button className="secondary-button" onClick={installCli} disabled={installingCli || sending}>
              {installingCli
                ? cliProgress?.stage === 'retrying' ? '正在重试…' : '正在更新…'
                : cliProgress?.stage === 'error' ? '重试更新' : '安全更新'}
            </button>
          </div>
          {cliProgress && (installingCli || cliProgress.stage === 'error') && (
            <div className="component-progress" aria-live="polite">
              <div className="progress-track">
                <div style={{ width: `${cliProgressPercent}%` }} />
              </div>
              <span>
                {cliProgress.message}
                {cliProgress.downloaded > 0 ? ` · ${formatBytes(cliProgress.downloaded)}${cliProgress.total ? ` / ${formatBytes(cliProgress.total)}` : ''}` : ''}
              </span>
              {cliAttemptLabel && <small className="component-progress-meta">{cliAttemptLabel}</small>}
            </div>
          )}
        </div>
      )}

      {!status?.authenticated && (
        <div className="connection-card">
          {!status ? (
            <>
              <strong>正在检测飞书连接组件…</strong>
              <span>首次启动可能需要几秒钟。</span>
            </>
          ) : !status.installed ? (
            <>
              <strong>下载飞书官方连接组件</strong>
              <span>组件不再塞进安装包，首次使用时单独下载，主程序更轻，组件也能独立更新。</span>
              <small className="component-security-note">来源：larksuite/cli（MIT）· 安装前强制核对官方 SHA-256</small>
              {cliProgress && (
                <div className={`component-progress ${cliProgress.stage === 'error' ? 'has-error' : ''}`} aria-live="polite">
                  <div className="progress-track">
                    <div style={{ width: `${cliProgressPercent}%` }} />
                  </div>
                  <span>
                    {cliProgress.message}
                    {cliProgress.downloaded > 0 ? ` · ${formatBytes(cliProgress.downloaded)}${cliProgress.total ? ` / ${formatBytes(cliProgress.total)}` : ''}` : ''}
                  </span>
                  {cliAttemptLabel && <small className="component-progress-meta">{cliAttemptLabel}</small>}
                </div>
              )}
              <div className="inline-actions">
                <button className="secondary-button" onClick={() => void checkStatus(true)} disabled={checking || installingCli}>
                  {checking ? '检测中…' : '重新检测'}
                </button>
                <button className="primary-button" onClick={installCli} disabled={installingCli}>
                  {installingCli
                    ? cliProgress?.stage === 'retrying' ? '正在重试…' : '正在安全下载…'
                    : cliProgress?.stage === 'error' ? '重试下载' : '下载官方组件'}
                </button>
              </div>
            </>
          ) : loginSession ? (
            <>
              <strong>请在浏览器中完成飞书授权</strong>
              {loginSession.userCode && (
                <span>如果网页要求输入授权码：<code className="auth-code">{loginSession.userCode}</code></span>
              )}
              <div className="inline-actions">
                <button className="secondary-button" onClick={openLoginPage}>
                  重新打开授权页
                </button>
                <button className="secondary-button" onClick={cancelLogin}>取消</button>
                <button className="primary-button" onClick={finishLogin} disabled={finishingLogin}>
                  {finishingLogin ? '正在确认…' : '我已完成授权'}
                </button>
              </div>
            </>
          ) : (
            <>
              <strong>尚未连接飞书</strong>
              <span>无需命令行，点击后按浏览器页面提示确认即可。</span>
              <button className="primary-button" onClick={startLogin} disabled={connecting}>
                {connecting ? '正在打开授权页…' : '连接飞书'}
              </button>
            </>
          )}
        </div>
      )}

      {status?.authenticated && <div className="form-grid">
        <label>
          接收位置
          <select value={destinationKind} onChange={(event) => setDestinationKind(event.target.value as typeof destinationKind)}>
            <option value="self">发给自己（自动获取 open_id）</option>
            <option value="user">指定用户 open_id</option>
            <option value="chat">指定群聊 chat_id</option>
          </select>
        </label>
        {destinationKind !== 'self' && (
          <label>
            {destinationKind === 'user' ? '用户 open_id' : '群聊 chat_id'}
            <input
              value={destinationId}
              onChange={(event) => setDestinationId(event.target.value)}
              placeholder={destinationKind === 'user' ? 'ou_xxx' : 'oc_xxx'}
            />
          </label>
        )}
      </div>}

      {status?.authenticated && <label className="checkbox-row">
        <input type="checkbox" checked={onlyPending} onChange={(event) => setOnlyPending(event.target.checked)} />
        只发送尚未成功发送的表情
      </label>}

      {status?.authenticated && (
        <div className={`send-summary ${targetCount > 0 ? 'ready' : ''}`}>
          <strong>
            {targetCount > 0
              ? onlyPending
                ? `检测到 ${targetCount} 个待发送表情`
                : `将重新发送全部 ${targetCount} 个表情`
              : '当前没有待发送表情'}
          </strong>
          <span>
            {targetCount > 0
              ? onlyPending
                ? '微信中新采集的表情会自动进入待发送列表，点击下方按钮即可发送到飞书。'
                : '已发送过的表情也会再次发送，请确认后继续。'
              : stickerCount > 0
                ? '如需重新发送已发过的表情，请取消勾选上方“只发送尚未成功发送的表情”。'
                : '请先从微信采集表情。'}
          </span>
        </div>
      )}

      {progress && (
        <div className="progress-block">
          <div className="progress-meta">
            <span>{progress.current} / {progress.total}</span>
            <span>成功 {progress.sent} · 失败 {progress.failed}</span>
          </div>
          <div className="progress-track">
            <div style={{ width: `${progress.total ? (progress.current / progress.total) * 100 : 0}%` }} />
          </div>
          {progress.message && <small>{progress.message}</small>}
        </div>
      )}

      {status?.authenticated && <div className="button-row">
        <button className="secondary-button" onClick={() => void checkStatus(true)} disabled={checking || sending || installingCli}>
          {checking ? '正在检查连接…' : connectionFeedback.checkedAt ? '再次检查连接' : '检查连接'}
        </button>
        <button
          className="primary-button"
          onClick={send}
          disabled={sending || !status?.authenticated || targetCount === 0}
        >
          {sending
            ? '正在发送…'
            : onlyPending
              ? `发送 ${targetCount} 个待发送表情`
              : `重新发送全部 ${targetCount} 个表情`}
        </button>
      </div>}
    </section>
  );
}
