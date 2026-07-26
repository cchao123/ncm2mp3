import { useEffect, useState } from "react";
import { api, Progress, QueueItem } from "../api";

/**
 * 下载队列页。
 *
 * ADR-0005：串行 + 随机间隔，上千首会跑数小时，所以进度必须随时看得清。
 * ADR-0009：失败不中断队列，跑完给清单；关窗即暂停，重开续传。
 */
export default function Queue({
  progress,
  setProgress,
}: {
  progress: Progress | null;
  setProgress: (p: Progress) => void;
}) {
  const [failed, setFailed] = useState<QueueItem[]>([]);
  const [error, setError] = useState("");

  const total = progress ? progress.done + progress.failed + progress.pending : 0;
  const finished = progress ? progress.done + progress.failed : 0;
  const pct = total ? Math.round((finished / total) * 100) : 0;

  useEffect(() => {
    api.getFailed().then(setFailed).catch(() => {});
  }, [progress?.failed]);

  async function start() {
    setError("");
    try {
      await api.startDownload();
      setProgress(await api.getProgress());
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <div className="topbar">
        <h1>下载队列</h1>
        <div className="spacer" />
        {progress?.running ? (
          <button onClick={() => api.stopDownload()}>暂停</button>
        ) : (
          <button className="primary" onClick={start} disabled={!progress?.pending}>
            开始下载
          </button>
        )}
        <button
          onClick={async () => {
            await api.clearFinished();
            setProgress(await api.getProgress());
            setFailed([]);
          }}
        >
          清空已完成
        </button>
      </div>

      <div className="content">
        {error && <div className="notice err">{error}</div>}

        <div className="card" style={{ marginBottom: 18 }}>
          <div className="stat-row">
            <span>
              待下载 <b>{progress?.pending ?? 0}</b>
            </span>
            <span style={{ color: "var(--ok)" }}>
              已完成 <b>{progress?.done ?? 0}</b>
            </span>
            <span style={{ color: progress?.failed ? "var(--err)" : undefined }}>
              失败 <b>{progress?.failed ?? 0}</b>
            </span>
          </div>

          <div className="progress-bar">
            <div style={{ width: `${pct}%` }} />
          </div>

          <div className="faint">
            {progress?.current
              ? `${progress.current} — ${progress.last_message}`
              : progress?.last_message || "队列空闲"}
          </div>
        </div>

        {!!progress?.pending && !progress.running && (
          <div className="notice">
            为降低账号风控风险，下载是串行的并带随机间隔，速度不快。
            上千首可能需要数小时。关闭窗口会暂停，下次打开可继续。
          </div>
        )}

        {failed.length > 0 && (
          <>
            <h2 style={{ fontSize: 14, margin: "20px 0 8px" }}>
              未能下载的曲目（{failed.length}）
            </h2>
            {failed.map((f) => (
              <div className="row" key={f.track_id}>
                <span className="title">{f.title}</span>
                <span className="sub">{f.artists}</span>
                <span className="faint" style={{ flexShrink: 0 }}>
                  {f.reason ?? "未知原因"}
                </span>
              </div>
            ))}
          </>
        )}
      </div>
    </>
  );
}
