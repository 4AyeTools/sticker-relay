export type WechatLoginState =
  | { state: 'idle' }
  | { state: 'restoring' }
  | { state: 'waiting' }
  | { state: 'scanned' }
  | { state: 'expired' }
  | { state: 'logged-in' };

export interface WechatQrResult {
  uuid: string;
  dataUrl: string;
}

export type FeishuSendState = 'pending' | 'sending' | 'sent' | 'failed';

export interface StickerRecord {
  id: string;
  sha256: string;
  wechatMd5?: string;
  filename: string;
  mimeType: string;
  bytes: number;
  sourceUrl?: string;
  importedAt: number;
  feishuState: FeishuSendState;
  feishuMessageId?: string;
  errorMessage?: string;
}

export interface StickerImportEvent {
  imported: number;
  skipped: number;
  unsupported: number;
  latest?: StickerRecord;
  warning?: string;
}

export interface FeishuCliStatus {
  installed: boolean;
  version?: string;
  authenticated: boolean;
  detail?: string;
  source?: 'managed' | 'bundled-legacy' | 'path' | string;
  executablePath?: string;
  latestVersion?: string;
  updateAvailable: boolean;
}

export interface FeishuCliProgress {
  stage: 'resolving' | 'downloading' | 'verifying' | 'retrying' | 'installing' | 'done' | 'error' | string;
  downloaded: number;
  total?: number;
  message: string;
  done: boolean;
  attempt?: number;
  maxAttempts?: number;
  source?: string;
}

export interface FeishuSelf {
  openId: string;
  name?: string;
}

export interface FeishuLoginSession {
  stage: 'config' | 'authorize';
  verificationUrl: string;
  userCode?: string;
  expiresAt?: number;
}

export interface FeishuLoginAdvance {
  status?: FeishuCliStatus;
  session?: FeishuLoginSession;
}

export interface StickerLibraryLocation {
  path: string;
  isDefault: boolean;
}

export interface StickerLibraryChangeResult extends StickerLibraryLocation {
  canceled: boolean;
  migratedCount: number;
}

export interface StickerDeleteResult {
  canceled: boolean;
  removed: number;
}

export type FeishuDestination =
  | { kind: 'self' }
  | { kind: 'user'; id: string }
  | { kind: 'chat'; id: string };

export interface FeishuSendRequest {
  destination: FeishuDestination;
  onlyPending: boolean;
}

export interface FeishuSendProgress {
  current: number;
  total: number;
  stickerId?: string;
  sent: number;
  failed: number;
  message?: string;
  done: boolean;
}

export interface ExportResult {
  canceled: boolean;
  path?: string;
  count?: number;
}

export interface DesktopApi {
  wechat: {
    requestQr(): Promise<WechatQrResult>;
    poll(uuid: string, tip: 0 | 1): Promise<WechatLoginState>;
    status(): Promise<WechatLoginState>;
    prepareExit(): Promise<void>;
    logout(): Promise<void>;
  };
  stickers: {
    list(): Promise<StickerRecord[]>;
    dataUrl(id: string): Promise<string | null>;
    exportZip(): Promise<ExportResult>;
    location(): Promise<StickerLibraryLocation>;
    chooseLocation(): Promise<StickerLibraryChangeResult>;
    openLocation(): Promise<void>;
    remove(ids: string[], clearAll?: boolean): Promise<StickerDeleteResult>;
  };
  feishu: {
    status(): Promise<FeishuCliStatus>;
    checkUpdate(): Promise<FeishuCliStatus>;
    installCli(): Promise<FeishuCliStatus>;
    self(): Promise<FeishuSelf>;
    startLogin(): Promise<FeishuLoginSession>;
    openLoginPage(): Promise<void>;
    finishLogin(): Promise<FeishuLoginAdvance>;
    cancelLogin(): Promise<void>;
    send(request: FeishuSendRequest): Promise<{ started: boolean; total: number }>;
  };
  events: {
    onStickerImported(callback: (event: StickerImportEvent) => void): () => void;
    onFeishuProgress(callback: (event: FeishuSendProgress) => void): () => void;
    onFeishuCliProgress(callback: (event: FeishuCliProgress) => void): () => void;
  };
}
