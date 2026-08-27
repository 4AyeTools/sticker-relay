import { useEffect, useState } from 'react';
import type { StickerLibraryLocation, StickerRecord } from '../shared/types';

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

function StickerImage({ id }: { id: string }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    void window.desktop.stickers.dataUrl(id).then((value) => {
      if (active) setSrc(value);
    });
    return () => { active = false; };
  }, [id]);
  return src ? <img src={src} alt="已采集表情" /> : <div className="image-placeholder">加载中</div>;
}

interface Props {
  stickers: StickerRecord[];
  onUpdated: () => Promise<void>;
  onNotice: (message: string) => void;
}

export default function StickerGrid({ stickers, onUpdated, onNotice }: Props) {
  const [location, setLocation] = useState<StickerLibraryLocation | null>(null);
  const [changingLocation, setChangingLocation] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    void window.desktop.stickers.location().then(setLocation).catch((error: unknown) => {
      onNotice(error instanceof Error ? error.message : String(error));
    });
  }, []);

  useEffect(() => {
    const selectable = new Set(
      stickers.filter((sticker) => sticker.feishuState !== 'sending').map((sticker) => sticker.id),
    );
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => selectable.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [stickers]);

  const chooseLocation = async () => {
    setChangingLocation(true);
    try {
      const result = await window.desktop.stickers.chooseLocation();
      setLocation({ path: result.path, isDefault: result.isDefault });
      if (!result.canceled) {
        await onUpdated();
        onNotice(result.migratedCount > 0
          ? `已迁移 ${result.migratedCount} 个表情到 ${result.path}`
          : `表情库位置保持为 ${result.path}`);
      }
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setChangingLocation(false);
    }
  };

  const openLocation = async () => {
    try {
      await window.desktop.stickers.openLocation();
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
    }
  };

  const selectableIds = stickers
    .filter((sticker) => sticker.feishuState !== 'sending')
    .map((sticker) => sticker.id);
  const allSelected = selectableIds.length > 0
    && selectableIds.every((id) => selectedIds.has(id));
  const hasSending = stickers.some((sticker) => sticker.feishuState === 'sending');

  const toggleSelected = (id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleSelectAll = () => {
    setSelectedIds(allSelected ? new Set() : new Set(selectableIds));
  };

  const removeStickers = async (ids: string[], clearAll = false) => {
    setDeleting(true);
    try {
      const result = await window.desktop.stickers.remove(ids, clearAll);
      if (!result.canceled) {
        setSelectedIds((current) => {
          const next = new Set(current);
          ids.forEach((id) => next.delete(id));
          return next;
        });
        await onUpdated();
        onNotice(result.removed > 0
          ? `已从本地表情库删除 ${result.removed} 个表情。微信收藏和飞书消息未受影响。`
          : '没有找到需要删除的本地表情。');
      }
    } catch (error) {
      onNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <section className="library-panel">
      <div className="library-heading">
        <div>
          <span className="step">03</span>
          <h2>表情小仓库</h2>
        </div>
        <span>{stickers.length} 个</span>
      </div>

      <div className="library-location">
        <div className="library-path" title={location?.path}>
          <span>{location?.isDefault ? '默认位置' : '自定义位置'}</span>
          <strong>{location?.path || '正在读取…'}</strong>
        </div>
        <div className="inline-actions">
          <button className="secondary-button compact-button" onClick={openLocation} disabled={!location}>
            打开文件夹
          </button>
          <button className="secondary-button compact-button" onClick={chooseLocation} disabled={changingLocation}>
            {changingLocation ? '正在迁移…' : '更改位置'}
          </button>
        </div>
      </div>

      {stickers.length > 0 && <div className="library-toolbar">
        <div className="selection-summary">
          <button
            className="secondary-button compact-button"
            onClick={toggleSelectAll}
            disabled={deleting || selectableIds.length === 0}
          >
            {allSelected ? '取消全选' : '全选'}
          </button>
          <span>已选择 <strong>{selectedIds.size}</strong> 个</span>
        </div>
        <div className="inline-actions">
          <button
            className="secondary-button compact-button danger-button"
            onClick={() => void removeStickers([...selectedIds])}
            disabled={deleting || selectedIds.size === 0}
          >
            {deleting ? '正在删除…' : '删除所选'}
          </button>
          <button
            className="secondary-button compact-button danger-button danger-button-strong"
            onClick={() => void removeStickers(stickers.map((sticker) => sticker.id), true)}
            disabled={deleting || hasSending}
            title={hasSending ? '有表情正在发送，请等待发送完成' : '清空全部本地表情'}
          >
            清空全部
          </button>
        </div>
      </div>}

      <div className="library-scroll">
      {stickers.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">◫</div>
          <strong>仓库还是空的</strong>
          <span>先连接微信，再把收藏表情发给文件传输助手。</span>
        </div>
      ) : (
        <div className="sticker-grid">
          {stickers.map((sticker) => (
            <article className={`sticker-card${selectedIds.has(sticker.id) ? ' selected' : ''}`} key={sticker.id}>
              <div className="sticker-image">
                <StickerImage id={sticker.id} />
                <button
                  className={`sticker-select-button${selectedIds.has(sticker.id) ? ' active' : ''}`}
                  type="button"
                  aria-label={selectedIds.has(sticker.id) ? '取消选择' : '选择表情'}
                  aria-pressed={selectedIds.has(sticker.id)}
                  disabled={deleting || sticker.feishuState === 'sending'}
                  onClick={() => toggleSelected(sticker.id)}
                >✓</button>
                <button
                  className="sticker-delete-button"
                  type="button"
                  aria-label="删除这个本地表情"
                  title={sticker.feishuState === 'sending' ? '发送完成后才能删除' : '删除本地表情'}
                  disabled={deleting || sticker.feishuState === 'sending'}
                  onClick={() => void removeStickers([sticker.id])}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <path d="M3.5 5h9M6 5V3.5h4V5m-5.5 0 .6 7.5h5.8l.6-7.5M6.7 7.2v3.2m2.6-3.2v3.2" />
                  </svg>
                </button>
              </div>
              <div className="sticker-info">
                <span>{sticker.mimeType.replace('image/', '').toUpperCase()} · {formatSize(sticker.bytes)}</span>
                <span className={`send-state ${sticker.feishuState}`}>{
                  sticker.feishuState === 'sent' ? '已发飞书'
                    : sticker.feishuState === 'sending' ? '发送中'
                      : sticker.feishuState === 'failed' ? '发送失败'
                        : '待发送'
                }</span>
              </div>
              {sticker.errorMessage && <div className="card-error" title={sticker.errorMessage}>{sticker.errorMessage}</div>}
            </article>
          ))}
        </div>
      )}
      </div>
    </section>
  );
}
