import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, Profile, Progress } from "./api";
import Login from "./pages/Login";
import Search from "./pages/Search";
import Queue from "./pages/Queue";
import Library from "./pages/Library";
import { PlayerBar, usePlayer } from "./player";

type Tab = "search" | "queue" | "library";

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [profile, setProfile] = useState<Profile | null>(null);
  const [tab, setTab] = useState<Tab>("search");
  const [progress, setProgress] = useState<Progress | null>(null);
  const [outDir, setOutDir] = useState("");
  // 播放器状态提在这一层：切页面时试听不该中断
  const player = usePlayer();

  const refreshAuth = useCallback(async () => {
    const ok = await api.authStatus();
    setAuthed(ok);
    if (ok) {
      try {
        setProfile(await api.getProfile());
      } catch {
        // 钥匙串里的凭据可能已被服务端登出。ADR-0004：此时引导重新扫码，
        // 而不是把一堆失败当网络错误反复重试。
        setAuthed(false);
        setProfile(null);
      }
    }
  }, []);

  useEffect(() => {
    refreshAuth();
    api.getOutputDir().then(setOutDir).catch(() => {});
    api.getProgress().then(setProgress).catch(() => {});
  }, [refreshAuth]);

  // 下载进度靠事件推送，避免轮询
  useEffect(() => {
    const un = listen<Progress>("download-progress", (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  if (authed === null) {
    return <div className="center-box muted">正在启动…</div>;
  }

  if (!authed) {
    return <Login onLoggedIn={refreshAuth} />;
  }

  const pendingBadge = progress && progress.pending > 0 ? progress.pending : null;

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">网易云音乐下载器</div>

        <div
          className={`nav-item ${tab === "search" ? "active" : ""}`}
          onClick={() => setTab("search")}
        >
          <span>⌕</span> 搜索
        </div>
        <div
          className={`nav-item ${tab === "queue" ? "active" : ""}`}
          onClick={() => setTab("queue")}
        >
          <span>↓</span> 下载队列
          {pendingBadge && <span className="badge">{pendingBadge}</span>}
        </div>
        <div
          className={`nav-item ${tab === "library" ? "active" : ""}`}
          onClick={() => setTab("library")}
        >
          <span>▤</span> 曲库
        </div>

        <div className="sidebar-foot">
          <div style={{ color: "var(--text-dim)", marginBottom: 4 }}>
            {profile?.nickname ?? ""}
          </div>
          <div
            style={{ cursor: "pointer" }}
            onClick={async () => {
              await api.logout();
              setAuthed(false);
              setProfile(null);
            }}
          >
            退出登录
          </div>
        </div>
      </aside>

      <main className="main">
        {/* 搜索后不跳转，以免打断连续搜索；只刷新侧边栏的待下载徽章 */}
        {tab === "search" && (
          <Search
            player={player}
            onQueued={async () => setProgress(await api.getProgress())}
          />
        )}
        {tab === "queue" && <Queue progress={progress} setProgress={setProgress} />}
        {tab === "library" && <Library outDir={outDir} />}

        <PlayerBar player={player} />
      </main>
    </div>
  );
}
