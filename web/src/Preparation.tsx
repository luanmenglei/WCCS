import { useEffect, useState } from "react";
import { z } from "zod";
import { api } from "./api";
import { useAccess } from "./access";
import type { Mutation } from "./SeatChecks";
const responseSchema = z.object({
  version: z.number(),
  profile: z.record(z.string(), z.unknown()).nullable(),
});
const competitionSchema = z.object({
  compiler_tool_id: z.string(),
  compiler_arguments: z.array(z.string()),
  judge_url: z.string(),
  contest_id: z.string(),
  language_id: z.string(),
});
const defaultCompetition = {
  compiler_tool_id: "gcc-cpp",
  compiler_arguments: ["-std=c++17", "-O2", "-Wall"],
  judge_url: "",
  contest_id: "",
  language_id: "cpp",
};
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
  let competition: z.infer<typeof competitionSchema> | null = null;
  try {
    const profile = z.record(z.string(), z.unknown()).parse(JSON.parse(text));
    const parsed = competitionSchema.safeParse(profile.competition);
    if (parsed.success) competition = parsed.data;
  } catch {
    /* The JSON editor displays errors when publishing. */
  }
  function updateCompetition(patch: Partial<typeof defaultCompetition> | null) {
    try {
      const profile = z.record(z.string(), z.unknown()).parse(JSON.parse(text));
      profile.competition =
        patch === null
          ? null
          : { ...defaultCompetition, ...competition, ...patch };
      setText(JSON.stringify(profile, null, 2));
      setError("");
    } catch {
      setError("请先修正维护配置 JSON，再编辑竞赛命令设置。");
    }
  }
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
            <input
              type="checkbox"
              checked={competition !== null}
              onChange={(event) =>
                updateCompetition(event.target.checked ? {} : null)
              }
            />
            启用 compilecpp 和 submit
          </label>
          {competition && (
            <>
              <label>
                编译器工具 ID
                <input
                  value={competition.compiler_tool_id}
                  onChange={(event) =>
                    updateCompetition({ compiler_tool_id: event.target.value })
                  }
                />
              </label>
              <label>
                编译参数，每行一个
                <textarea
                  rows={3}
                  value={competition.compiler_arguments.join("\n")}
                  onChange={(event) =>
                    updateCompetition({
                      compiler_arguments: event.target.value.split("\n"),
                    })
                  }
                />
              </label>
              <label>
                DOMjudge HTTPS 地址
                <input
                  type="url"
                  value={competition.judge_url}
                  onChange={(event) =>
                    updateCompetition({ judge_url: event.target.value })
                  }
                />
              </label>
              <label>
                DOMjudge 比赛 ID
                <input
                  value={competition.contest_id}
                  onChange={(event) =>
                    updateCompetition({ contest_id: event.target.value })
                  }
                />
              </label>
              <label>
                默认语言 ID
                <input
                  value={competition.language_id}
                  onChange={(event) =>
                    updateCompetition({ language_id: event.target.value })
                  }
                />
              </label>
              <p>
                导入竞赛脚本包后发布，再执行脚本部署。普通账户桌面将出现“比赛终端”，编译器路径取自当前工具配置。submit
                使用已核对的 DOMjudge IP 账号。
              </p>
            </>
          )}
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
