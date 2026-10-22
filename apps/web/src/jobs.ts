import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, useBackend } from "./api";
import type { DocId, JobInfo } from "./protocol/generated";

/**
 * A job of `doc` kept up to date from its events. Once it finishes the
 * document's cached answers are dropped: checking records new diagnostics,
 * validation learns object types.
 */
export function useJob(doc: DocId): [JobInfo | null, (j: JobInfo | null) => void] {
  const backend = useBackend();
  const qc = useQueryClient();
  const [job, setJobState] = useState<JobInfo | null>(null);
  const current = useRef<number | null>(null);
  const setJob = useCallback(
    (j: JobInfo | null) => {
      current.current = j?.job ?? null;
      setJobState(j);
      // A short job may have finished (and sent jobDone) before its start was answered.
      if (j?.state === "running") {
        void api.jobStatus(backend, j.job).then((now) => {
          if (now.state !== "running" && current.current === now.job) {
            setJobState(now);
            void qc.invalidateQueries({ queryKey: ["doc", doc] });
          }
        });
      }
    },
    [backend, qc, doc],
  );
  useEffect(
    () =>
      backend.onEvent((ev) => {
        if (ev.type === "jobProgress" && ev.job === current.current) {
          setJobState((j) => (j && j.job === ev.job ? { ...j, done: ev.done, total: ev.total } : j));
        } else if (ev.type === "jobDone" && ev.info.job === current.current) {
          setJobState(ev.info);
          void qc.invalidateQueries({ queryKey: ["doc", doc] });
        }
      }),
    [backend, qc, doc],
  );
  return [job, setJob];
}
