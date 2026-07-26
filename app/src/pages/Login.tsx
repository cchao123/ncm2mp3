import { useEffect, useRef, useState } from "react";
import { api } from "../api";

/**
 * 扫码登录页。
 *
 * unikey 约 3 分钟过期，这里在收到 800 时自动换一张新码——
 * 让用户什么时候回到电脑前都能扫，不必掐时间。
 */
export default function Login({ onLoggedIn }: { onLoggedIn: () => void }) {
  const [svg, setSvg] = useState("");
  const [status, setStatus] = useState("正在生成二维码…");
  const [error, setError] = useState("");
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    let timer: number | undefined;

    async function cycle() {
      while (alive.current) {
        let unikey: string;
        try {
          const r = await api.qrStart();
          unikey = r.unikey;
          setSvg(r.svg);
          setError("");
          setStatus("请用网易云音乐 App 扫码");
        } catch (e) {
          setError(String(e));
          return;
        }

        // 轮询直到成功或过期
        while (alive.current) {
          await new Promise<void>((r) => {
            timer = window.setTimeout(r, 2000);
          });
          if (!alive.current) return;

          try {
            const p = await api.qrPoll(unikey);
            setStatus(p.message);
            if (p.logged_in) {
              onLoggedIn();
              return;
            }
            if (p.code === 800) break; // 过期，跳出去换一张新码
          } catch (e) {
            setError(String(e));
            return;
          }
        }
      }
    }

    cycle();
    return () => {
      alive.current = false;
      if (timer) window.clearTimeout(timer);
    };
  }, [onLoggedIn]);

  return (
    <div className="center-box">
      <div>
        <div style={{ fontSize: 17, fontWeight: 600, marginBottom: 6 }}>
          网易云音乐下载器
        </div>
        <div className="faint">登录后可下载你账号权限内的曲目</div>
      </div>

      <div className="qr" dangerouslySetInnerHTML={{ __html: svg }} />

      <div>
        <div style={{ marginBottom: 6 }}>{status}</div>
        <div className="faint">二维码过期会自动更换，不用赶时间</div>
      </div>

      {error && <div className="notice err">{error}</div>}
    </div>
  );
}
