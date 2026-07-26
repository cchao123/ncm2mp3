import { invoke } from "@tauri-apps/api/core";

/**
 * 曲目可用性（ADR-0007）。
 * 区分它们是为了在点击之前就说清楚哪些下不了，而不是让用户试错。
 * 按 ADR-0001，非 downloadable 的一律不下载、不绕过。
 */
export type Availability =
  | "downloadable"
  | "vip_only"
  | "pay_album"
  | "unavailable";

export const AVAILABILITY_LABEL: Record<Availability, string> = {
  downloadable: "",
  vip_only: "需会员",
  pay_album: "需购买",
  unavailable: "无版权",
};

export interface Track {
  id: number;
  title: string;
  artists: string;
  album: string;
  duration_ms: number;
  cover_url: string;
  availability: Availability;
}

export interface Profile {
  user_id: number;
  nickname: string;
  vip_type: number;
  avatar_url: string;
}

export interface Progress {
  current: string | null;
  done: number;
  failed: number;
  pending: number;
  running: boolean;
  last_message: string;
}

export interface QueueItem {
  track_id: number;
  title: string;
  artists: string;
  album: string;
  status: string;
  reason: string | null;
}

export interface TrackRecord {
  track_id: number;
  title: string;
  artists: string;
  album: string;
  file_path: string;
  file_size: number;
  bitrate: number;
  created_at: number;
}

export const api = {
  authStatus: () => invoke<boolean>("auth_status"),
  qrStart: () => invoke<{ unikey: string; svg: string }>("qr_start"),
  qrPoll: (unikey: string) =>
    invoke<{ code: number; message: string; logged_in: boolean }>("qr_poll", { unikey }),
  logout: () => invoke<void>("logout"),

  getProfile: () => invoke<Profile>("get_profile"),
  searchSongs: (keyword: string, limit = 30, offset = 0) =>
    invoke<{ tracks: Track[]; total: number }>("search_songs", {
      keyword,
      limit,
      offset,
    }),
  /** 试听直链（标准音质，非下载用的 320k） */
  previewUrl: (trackId: number) => invoke<string>("preview_url", { trackId }),
  filterDownloaded: (trackIds: number[]) =>
    invoke<number[]>("filter_downloaded", { trackIds }),

  enqueue: (tracks: Track[]) => invoke<number>("enqueue", { tracks }),
  startDownload: () => invoke<void>("start_download"),
  stopDownload: () => invoke<void>("stop_download"),
  getProgress: () => invoke<Progress>("get_progress"),
  getFailed: () => invoke<QueueItem[]>("get_failed"),
  clearFinished: () => invoke<void>("clear_finished"),

  getLibrary: () => invoke<TrackRecord[]>("get_library"),
  getOutputDir: () => invoke<string>("get_output_dir"),
  openOutputDir: () => invoke<void>("open_output_dir"),
  revealFile: (path: string) => invoke<void>("reveal_file", { path }),
};

export function fmtDuration(ms: number): string {
  if (!ms) return "--:--";
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}
