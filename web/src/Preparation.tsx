import { useEffect, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import { useAccess } from "./access";
import type { Mutation } from "./SeatChecks";
const responseSchema = z.object({
  version: z.number(),
  profile: z.record(z.string(), z.unknown()).nullable(),
});
export function Preparation({
  toolVersion,
  disabled,
  onMutate,
  onVersion,
}: {
  toolVersion: number;
  disabled: boolean;
  onMutate: Mutation;
  onVersion: (version: number) => void;
}) {
  const access = useAccess();
  const [version, setVersion] = useState(0);
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function reload() {
    const value = responseSchema.parse(await api("preparation"));
    setVersion(value.version);
    onVersion(value.version);
    setText(
      JSON.stringify(
        value.profile ?? {
          schema_version: 1,
          tool_version: toolVersion,
          work_directory: "D:\\Contest",
          desktop_directory: "C:\\Users\\WccsContest\\Desktop",
          script_directory: "C:\\ProgramData\\WCCS\\Scripts",
          links: [],
          package: null,
        },
        null,
        2,
      ),
    );
  }
  useEffect(() => {
    let active = true;
    void api("preparation")
      .then(responseSchema.parse)
      .then((value) => {
        if (active) {
          setVersion(value.version);
          onVersion(value.version);
          setText(
            JSON.stringify(
              value.profile ?? {
                schema_version: 1,
                tool_version: toolVersion,
                work_directory: "D:\\Contest",
                desktop_directory: "C:\\Users\\WccsContest\\Desktop",
                script_directory: "C:\\ProgramData\\WCCS\\Scripts",
                links: [],
                package: null,
              },
              null,
              2,
            ),
          );
        }
      })
      .catch((error) => {
        if (active)
          setError(error instanceof Error ? error.message : "加载失败");
      });
    return () => {
      active = false;
    };
  }, [toolVersion, onVersion]);
  return (
    <section className="panel operations-panel">
      <details>
        <summary>桌面与脚本维护配置 · v{version}</summary>
        <p>
          核对示例路径后保存。desktop_directory
          必须对应现有普通比赛账户桌面，工作目录与该账户配置一致。工具入口引用工具
          ID，路径和版本取自工具配置。
        </p>
        <p>
          links 支持 tool（name、tool_id）、website（name、url）和
          resource（name、path）。入口写入桌面 WCCS
          文件夹。脚本清单包含固定运行依赖、来源、许可、版本及逐文件摘要。
        </p>
        <fieldset
          disabled={disabled || busy || !access.manage || !access.allRooms}
        >
          <label>
            版本化配置 JSON
            <textarea
              rows={16}
              value={text}
              onChange={(e) => setText(e.target.value)}
              spellCheck={false}
            />
          </label>
          <label>
            导入配置文件
            <input
              type="file"
              accept=".json"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file)
                  void file
                    .text()
                    .then((value) => {
                      if (value.length > 250000)
                        throw new Error("配置文件过大");
                      JSON.parse(value);
                      setText(value);
                    })
                    .catch((error) => setError(String(error)));
                e.target.value = "";
              }}
            />
          </label>
          <label>
            导入竞赛脚本包清单
            <input
              type="file"
              accept=".json"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file)
                  void file
                    .text()
                    .then((value) => {
                      if (value.length > 200000)
                        throw new Error("脚本清单过大");
                      setText(
                        JSON.stringify(
                          { ...JSON.parse(text), package: JSON.parse(value) },
                          null,
                          2,
                        ),
                      );
                    })
                    .catch((error) => setError(String(error)));
                e.target.value = "";
              }}
            />
          </label>
          <button
            onClick={() => {
              setError("");
              let profile: unknown;
              try {
                profile = JSON.parse(text);
              } catch {
                setError("JSON 格式无效");
                return;
              }
              setBusy(true);
              void onMutate(
                "preparation",
                { base_version: version, profile },
                "维护配置已发布，请重新预览目标后执行。",
              )
                .then(reload)
                .catch((error) =>
                  setError(error instanceof Error ? error.message : "保存失败"),
                )
                .finally(() => setBusy(false));
            }}
          >
            发布维护配置
          </button>
        </fieldset>
        <button
          disabled={busy}
          onClick={() =>
            void reload().catch((error) => setError(String(error)))
          }
        >
          重新加载
        </button>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </details>
    </section>
  );
}
