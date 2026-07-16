"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { TrafficBudgetStatus } from "@/types";
import { RippleButton } from "./ui/ripple";

const GIB = 1024 * 1024 * 1024;
const MIB = 1024 * 1024;
const POLL_MS = 5000;

function formatBytes(bytes: number): string {
  if (bytes >= GIB) return `${(bytes / GIB).toFixed(2)} GB`;
  if (bytes >= MIB) return `${(bytes / MIB).toFixed(1)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

export function TrafficBudgetCard() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<TrafficBudgetStatus | null>(null);
  const [limitGb, setLimitGb] = useState("");
  const [spikeMb, setSpikeMb] = useState("");
  const [isSaving, setIsSaving] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<TrafficBudgetStatus>("get_traffic_budget"));
    } catch {
      // A failed poll is not worth a toast; the next tick retries.
    }
  }, []);

  useEffect(() => {
    void refresh();
    const id = setInterval(() => {
      void refresh();
    }, POLL_MS);
    return () => {
      clearInterval(id);
    };
  }, [refresh]);

  // Seed the inputs from whatever is already saved, once.
  useEffect(() => {
    void (async () => {
      try {
        const s = await invoke<TrafficBudgetStatus>("get_traffic_budget");
        if (s.limitBytes) setLimitGb((s.limitBytes / GIB).toString());
        if (s.spikeBytesPerMin)
          setSpikeMb((s.spikeBytesPerMin / MIB).toString());
      } catch {
        // Leave the inputs empty; saving still works.
      }
    })();
  }, []);

  const handleSave = useCallback(async () => {
    const limit = Number.parseFloat(limitGb.replace(",", "."));
    const spike = Number.parseFloat(spikeMb.replace(",", "."));
    setIsSaving(true);
    try {
      setStatus(
        await invoke<TrafficBudgetStatus>("set_traffic_limits", {
          limitBytes:
            Number.isFinite(limit) && limit > 0
              ? Math.round(limit * GIB)
              : null,
          spikeBytesPerMin:
            Number.isFinite(spike) && spike > 0
              ? Math.round(spike * MIB)
              : null,
        }),
      );
      showSuccessToast(t("traffic.saved"));
    } catch (error) {
      showErrorToast(String(error));
    } finally {
      setIsSaving(false);
    }
  }, [limitGb, spikeMb, t]);

  const handleReset = useCallback(async () => {
    try {
      setStatus(await invoke<TrafficBudgetStatus>("reset_traffic_budget"));
      showSuccessToast(t("traffic.resetDone"));
    } catch (error) {
      showErrorToast(String(error));
    }
  }, [t]);

  const used = status?.usedBytes ?? 0;
  const limit = status?.limitBytes ?? null;
  const percent = limit && limit > 0 ? Math.min(100, (used / limit) * 100) : 0;
  const nearLimit = percent >= 80;

  return (
    <div className="space-y-3 border-t pt-3">
      <div>
        <Label className="text-sm font-medium">{t("traffic.title")}</Label>
        <p className="pt-1 text-xs text-muted-foreground">
          {t("traffic.description")}
        </p>
      </div>

      <div className="space-y-1">
        <div className="flex items-baseline justify-between text-xs">
          <span className="font-mono">
            {limit
              ? t("traffic.usedOf", {
                  used: formatBytes(used),
                  limit: formatBytes(limit),
                })
              : t("traffic.usedNoLimit", { used: formatBytes(used) })}
          </span>
          <span className="font-mono text-muted-foreground">
            {t("traffic.rate", { rate: formatBytes(status?.bytesPerMin ?? 0) })}
          </span>
        </div>
        {limit ? (
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
            <div
              className={cn(
                "h-full rounded-full transition-all",
                status?.limitReached
                  ? "bg-destructive"
                  : nearLimit
                    ? "bg-warning"
                    : "bg-success",
              )}
              style={{ width: `${percent}%` }}
            />
          </div>
        ) : null}
        {status?.limitReached ? (
          <p className="text-xs text-destructive">{t("traffic.stopped")}</p>
        ) : null}
      </div>

      <div className="grid grid-cols-2 gap-2">
        <div className="grid gap-1">
          <Label htmlFor="traffic-limit" className="text-xs">
            {t("traffic.limitLabel")}
          </Label>
          <Input
            id="traffic-limit"
            inputMode="decimal"
            value={limitGb}
            onChange={(e) => {
              setLimitGb(e.target.value);
            }}
            placeholder={t("traffic.limitPlaceholder")}
            className="h-8 text-xs"
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor="traffic-spike" className="text-xs">
            {t("traffic.spikeLabel")}
          </Label>
          <Input
            id="traffic-spike"
            inputMode="decimal"
            value={spikeMb}
            onChange={(e) => {
              setSpikeMb(e.target.value);
            }}
            placeholder={t("traffic.spikePlaceholder")}
            className="h-8 text-xs"
          />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2">
        <RippleButton
          size="sm"
          className="text-xs"
          disabled={isSaving}
          onClick={() => {
            void handleSave();
          }}
        >
          {t("traffic.saveButton")}
        </RippleButton>
        <RippleButton
          size="sm"
          variant="outline"
          className="text-xs"
          onClick={() => {
            void handleReset();
          }}
        >
          {t("traffic.resetButton")}
        </RippleButton>
      </div>
      <p className="text-xs text-muted-foreground">{t("traffic.resetHint")}</p>
    </div>
  );
}
