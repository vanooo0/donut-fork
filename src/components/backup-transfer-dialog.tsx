"use client";

import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface BackupTransferDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

const BACKUP_FILTER = { name: "Donut backup", extensions: ["donutbak"] };
const MIN_PASSWORD_LEN = 8;

export function BackupTransferDialog({
  isOpen,
  onClose,
}: BackupTransferDialogProps) {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [isExporting, setIsExporting] = useState(false);
  const [isImporting, setIsImporting] = useState(false);

  const busy = isExporting || isImporting;
  const passwordTooShort = password.length < MIN_PASSWORD_LEN;

  const handleClose = useCallback(() => {
    if (busy) return;
    setPassword("");
    onClose();
  }, [busy, onClose]);

  const handleExport = useCallback(async () => {
    const dest = await save({
      title: t("backup.saveTitle"),
      defaultPath: "donut-backup.donutbak",
      filters: [BACKUP_FILTER],
    });
    if (!dest) return;

    setIsExporting(true);
    try {
      await invoke("export_backup_file", { destPath: dest, password });
      showSuccessToast(t("backup.exportDone"));
      setPassword("");
      onClose();
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsExporting(false);
    }
  }, [password, onClose, t]);

  const handleImport = useCallback(async () => {
    const src = await open({
      title: t("backup.openTitle"),
      multiple: false,
      directory: false,
      filters: [BACKUP_FILTER],
    });
    if (!src || typeof src !== "string") return;

    setIsImporting(true);
    try {
      await invoke("import_backup_file", { srcPath: src, password });
      showSuccessToast(t("backup.importDone"));
      // Managers cache state in memory; a restart reloads everything the
      // backup just replaced on disk.
      await invoke("restart_application");
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
      setIsImporting(false);
    }
  }, [password, t]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("backup.title")}</DialogTitle>
          <DialogDescription>{t("backup.description")}</DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-2">
          <div className="grid gap-2">
            <Label htmlFor="backup-password">{t("backup.password")}</Label>
            <Input
              id="backup-password"
              type="password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
              }}
              placeholder={t("backup.passwordPlaceholder")}
              autoComplete="new-password"
              disabled={busy}
            />
            <p className="text-xs text-muted-foreground">
              {t("backup.passwordHint")}
            </p>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <LoadingButton
              isLoading={isExporting}
              disabled={busy || passwordTooShort}
              onClick={() => {
                void handleExport();
              }}
            >
              {t("backup.exportButton")}
            </LoadingButton>
            <LoadingButton
              isLoading={isImporting}
              disabled={busy || passwordTooShort}
              variant="outline"
              onClick={() => {
                void handleImport();
              }}
            >
              {t("backup.importButton")}
            </LoadingButton>
          </div>

          <p className="text-xs text-muted-foreground">
            {t("backup.importWarning")}
          </p>
        </div>
      </DialogContent>
    </Dialog>
  );
}
