import { useEffect, useRef, useState } from 'react';

type Phase = 'idle' | 'restoring' | 'loading' | 'waiting' | 'scanned' | 'logged-in' | 'expired' | 'error';

interface Props {
  stickerCount: number;
  onNotice: (message: string) => void;
}

export default function WechatPanel({ stickerCount, onNotice }: Props) {
  const [phase, setPhase] = useState<Phase>('idle');
  const [uuid, setUuid] = useState<string | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [error, setError] = useState('');
  const polling = useRef(false);
  const lastConnectionState = useRef<'idle' | 'restoring' | 'logged-in'>('idle');

  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;

    const refreshStatus = async () => {
      try {
        const result = await window.desktop.wechat.status();
        if (stopped || (result.state !== 'idle' && result.state !== 'restoring' && result.state !== 'logged-in')) return;
        const previous = lastConnectionState.current;
        lastConnectionState.current = result.state;
        setPhase((current) => {
          if (result.state === 'logged-in') return 'logged-in';
          if (result.state === 'restoring' && (current === 'idle' || current === 'restoring' || current === 'logged-in')) {
            return 'restoring';
          }
          if (result.state === 'idle' && (current === 'logged-in' || current === 'restoring')) return 'idle';
          return current;
        });
        if (result.state === 'logged-in' && previous !== 'logged-in') {
          onNotice('微信登录已恢复，正在继续监听文件传输助手。');
        }
        if (result.state === 'idle' && (previous === 'logged-in' || previous === 'restoring')) {
          onNotice('微信登录已失效，请重新扫码连接。');
        }
      } catch {
        // 状态检查失败不会打断正在进行的扫码或消息收取。
      } finally {
        if (!stopped) timer = window.setTimeout(refreshStatus, 5_000);
      }
    };

    void refreshStatus();
    return () => {
      stopped = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [onNotice]);

  const requestQr = async () => {
    setPhase('loading');
    setError('');
    try {
      const result = await window.desktop.wechat.requestQr();
      setUuid(result.uuid);
      setQrDataUrl(result.dataUrl);
      setPhase('waiting');
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setPhase('error');
    }
  };

  useEffect(() => {
    if (!uuid || (phase !== 'waiting' && phase !== 'scanned')) return undefined;
    let stopped = false;
    let timer: number | undefined;

    const poll = async () => {
      if (stopped || polling.current) return;
      polling.current = true;
      try {
        const result = await window.desktop.wechat.poll(uuid, phase === 'scanned' ? 1 : 0);
        if (stopped) return;
        if (result.state === 'scanned') setPhase('scanned');
        if (result.state === 'expired') setPhase('expired');
        if (result.state === 'logged-in') {
          lastConnectionState.current = 'logged-in';
          setPhase('logged-in');
          setQrDataUrl(null);
          onNotice('微信文件传输助手已连接，把收藏表情发过来就会自动收好。');
          return;
        }
      } catch (reason) {
        if (!stopped) {
          setError(reason instanceof Error ? reason.message : String(reason));
          setPhase('error');
        }
        return;
      } finally {
        polling.current = false;
      }
      timer = window.setTimeout(poll, 1_500);
    };
    void poll();
    return () => {
      stopped = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [uuid, phase, onNotice]);

  const logout = async () => {
    try {
      await window.desktop.wechat.logout();
      lastConnectionState.current = 'idle';
      setUuid(null);
      setQrDataUrl(null);
      setPhase('idle');
      onNotice('已退出网页版微信，并清除本机保存的登录状态。');
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setPhase('error');
    }
  };

  const statusText = {
    idle: '尚未登录',
    restoring: '正在恢复上次登录…',
    loading: '正在获取二维码…',
    waiting: '请使用手机微信扫码',
    scanned: '已扫码，请在手机上确认登录',
    'logged-in': '已登录，正在监听文件传输助手',
    expired: '二维码已过期，请刷新',
    error: error || '微信连接失败',
  }[phase];

  return (
    <section className="panel">
      <div className="panel-heading">
        <div>
          <span className="step">01</span>
          <h2>从微信收表情</h2>
        </div>
        <span className={`status-dot ${phase === 'logged-in' ? 'online' : ''}`}>{statusText}</span>
      </div>

      <p className="panel-description">
        连接文件传输助手后，把收藏表情从手机发过来，表情递会自动下载、去重并收进本地。
      </p>

      {qrDataUrl && (
        <div className="qr-wrap">
          <img src={qrDataUrl} alt="微信登录二维码" />
        </div>
      )}

      {phase === 'logged-in' && (
        <div className="success-box">
          <strong>正在收取</strong>
          <span>仓库里已有 {stickerCount} 个表情，可以继续从手机发送。</span>
        </div>
      )}

      {phase === 'restoring' && (
        <div className="success-box restoring-box">
          <strong>正在接上次的连接</strong>
          <span>登录状态已从本机安全存储恢复，正在校验微信会话。</span>
        </div>
      )}

      <div className="button-row">
        {phase !== 'logged-in' && phase !== 'restoring' && (
          <button className="primary-button" onClick={requestQr} disabled={phase === 'loading'}>
            {phase === 'idle' ? '扫码连接微信' : '刷新二维码'}
          </button>
        )}
        {phase === 'logged-in' && <button className="secondary-button" onClick={logout}>退出网页版微信</button>}
      </div>
    </section>
  );
}
