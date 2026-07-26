import { useEffect, useMemo, useState } from "react";
import { api, fmtSize, TrackRecord } from "../api";

/**
 * 曲库页。
 *
 * ADR-0008 把「方便使用」这一需求交给了这里：目录是平铺的，上千个文件摊在
 * 一个 Finder 窗口里并不好用，所以可搜索、可排序的列表 + 一键定位是必做项，
 * 不是锦上添花。「打开目录」按钮保留为逃生口。
 */
type SortKey = "time" | "title" | "artists" | "size";

export default function Library({ outDir }: { outDir: string }) {
  const [rows, setRows] = useState<TrackRecord[]>([]);
  const [q, setQ] = useState("");
  const [sort, setSort] = useState<SortKey>("time");
  const [error, setError] = useState("");

  useEffect(() => {
    api.getLibrary().then(setRows).catch((e) => setError(String(e)));
  }, []);

  const view = useMemo(() => {
    const kw = q.trim().toLowerCase();
    const filtered = kw
      ? rows.filter(
          (r) =>
            r.title.toLowerCase().includes(kw) ||
            r.artists.toLowerCase().includes(kw) ||
            r.album.toLowerCase().includes(kw)
        )
      : rows;

    const sorted = [...filtered];
    sorted.sort((a, b) => {
      switch (sort) {
        case "title":
          return a.title.localeCompare(b.title, "zh");
        case "artists":
          return a.artists.localeCompare(b.artists, "zh");
        case "size":
          return b.file_size - a.file_size;
        default:
          return b.created_at - a.created_at;
      }
    });
    return sorted;
  }, [rows, q, sort]);

  return (
    <>
      <div className="topbar">
        <h1>曲库</h1>
        <span className="faint">{rows.length} 首</span>
        <div className="spacer" />
        <input
          placeholder="搜索歌名 / 歌手 / 专辑"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          style={{
            background: "var(--bg-hover)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            padding: "7px 11px",
            color: "var(--text)",
            width: 220,
            font: "inherit",
          }}
        />
        <select
          value={sort}
          onChange={(e) => setSort(e.target.value as SortKey)}
          style={{
            background: "var(--bg-hover)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            padding: "7px 9px",
            color: "var(--text)",
            font: "inherit",
          }}
        >
          <option value="time">最近下载</option>
          <option value="title">按歌名</option>
          <option value="artists">按歌手</option>
          <option value="size">按大小</option>
        </select>
        <button onClick={() => api.openOutputDir()}>打开目录</button>
      </div>

      <div className="content">
        {error && <div className="notice err">{error}</div>}

        {outDir && (
          <div className="faint" style={{ marginBottom: 12 }}>
            输出目录：{outDir}
          </div>
        )}

        {!rows.length && !error && (
          <div className="muted">还没有下载任何曲目。去「我的歌单」选一个开始吧。</div>
        )}

        {view.map((r) => (
          <div
            className="row"
            key={r.track_id}
            onDoubleClick={() => api.revealFile(r.file_path)}
            title="双击在访达中定位"
          >
            <span className="title">{r.title}</span>
            <span className="sub">{r.artists}</span>
            <span className="sub">{r.album}</span>
            <span className="tail">{Math.round(r.bitrate / 1000)}k</span>
            <span className="tail" style={{ width: 64, textAlign: "right" }}>
              {fmtSize(r.file_size)}
            </span>
            <button
              onClick={() => api.revealFile(r.file_path)}
              style={{ padding: "4px 9px", fontSize: 12 }}
            >
              定位
            </button>
          </div>
        ))}
      </div>
    </>
  );
}
