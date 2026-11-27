// Settings → Integrações → Simkl: "Conectar" opens simkl.com with a code already filled in (the
// Nuvio Astra app is built in), then what Astra does with the account. The token stays in Rust,
// encrypted, and survives app updates.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import type { DeviceCode, SimklSettings, SimklStatus } from "../types";
import { locale, t } from "../i18n";

export function SimklSection({ settings, onChange }: { settings: SimklSettings; onChange(change: Partial<SimklSettings>): void }) {
  const [status, setStatus] = useState<SimklStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [device, setDevice] = useState<DeviceCode | null>(null);
  const poll = useRef(0);

  useEffect(() => {
    api.simklStatus().then(setStatus, (e) => setError(errorText(e)));
    return () => {
      window.clearTimeout(poll.current);
      void api.simklDeviceCancel();
    };
  }, []);

  const run = async (what: string, action: () => Promise<void>) => {
    setBusy(what);
    setError(null);
    setNote(null);
    try {
      await action();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const stopPolling = () => {
    window.clearTimeout(poll.current);
    setDevice(null);
  };

  const waitFor = (code: DeviceCode, interval: number, until: number) => {
    poll.current = window.setTimeout(async () => {
      if (Date.now() > until) {
        stopPolling();
        setError(t("O código expirou. Peça outro."));
        return;
      }
      try {
        const answer = await api.simklDevicePoll();
        if (answer.state === "pending") return waitFor(code, interval, until);
        if (answer.state === "slowDown") return waitFor(code, interval + 5, until);
        stopPolling();
        if (answer.state === "connected") {
          setStatus(await api.simklStatus());
          setNote(answer.username ? t("Conectado como {name}.", { name: answer.username }) : t("Conectado ao Simkl."));
        } else if (answer.state === "expired") setError(t("O código expirou. Peça outro."));
        else setError(t(answer.message ?? "Não foi possível conectar ao Simkl."));
      } catch (e) {
        stopPolling();
        setError(errorText(e));
      }
    }, interval * 1000);
  };

  const connect = () => run("connect", async () => {
    const code = await api.simklDeviceStart(true);
    setDevice(code);
    waitFor(code, code.interval, Date.now() + code.expiresIn * 1000);
  });

  const disconnect = () => {
    if (!window.confirm(t("Desconectar o Simkl deste perfil?"))) return;
    void run("disconnect", async () => setStatus(await api.simklDisconnect()));
  };

  const importNow = () => run("import", async () => {
    const added = await api.simklImport();
    setStatus(await api.simklStatus());
    setNote(added > 0 ? t("{n} item(ns) marcados como assistidos.", { n: added }) : t("Nada novo para importar."));
  });

  return (
    <div className="settings-group" data-testid="simkl-settings">
      <p className="muted">
        {t("O Simkl guarda o que você assiste (filmes, séries e animes). Conectado, o Astra avisa o Simkl do que está tocando, envia o que você marca como assistido e pode trazer seu histórico.")}
      </p>
      {status && !status.configured ? (
        <div className="row-error" data-testid="simkl-unavailable">{t("Esta versão do Nuvio Astra foi gerada sem o Simkl.")}</div>
      ) : status?.connected ? (
        <Row title={status.username ? t("Conectado como {name}", { name: status.username }) : t("Conectado ao Simkl")} description={t("Vale só para este perfil e continua conectado depois das atualizações.")}>
          <button className="ghost" onClick={disconnect} disabled={busy !== null} data-testid="simkl-disconnect">{t("Desconectar")}</button>
        </Row>
      ) : device ? (
        <div className="device-code" data-testid="simkl-device">
          <div className="muted small">{t("Abrimos o Simkl no navegador com o código já preenchido. Se pedir, digite:")}</div>
          <div className="device-code-value" data-testid="simkl-code">{device.userCode}</div>
          <div className="muted small"><span className="spinner" aria-hidden /> {t("Aguardando você autorizar no Simkl…")}</div>
          <button className="link" onClick={() => { stopPolling(); void api.simklDeviceCancel(); }}>{t("Cancelar")}</button>
        </div>
      ) : (
        <Row title={t("Conta do Simkl")} description={t("Conecte sua conta para o Simkl acompanhar o que você assiste.")}>
          <button className="primary" onClick={() => void connect()} disabled={busy !== null || !status} data-testid="simkl-connect">{busy === "connect" ? t("Aguarde…") : t("Conectar")}</button>
        </Row>
      )}
      {error && <div className="row-error" role="alert">{t(error)}</div>}
      {note && <div className="muted small" role="status" data-testid="simkl-note">{note}</div>}

      <h3 className="settings-subtitle">{t("O que o Astra faz com o Simkl")}</h3>
      <Row title={t("Avisar o que está tocando (scrobble)")} description={t("Começar, pausar e parar vão para o Simkl; a partir de 80% o título conta como assistido lá.")}>
        <Toggle label={t("Avisar o que está tocando (scrobble)")} checked={settings.scrobble} onChange={(v) => onChange({ scrobble: v })} />
      </Row>
      <Row title={t("Enviar o que eu marco como assistido")} description={t("“Marcar como assistido” (e desmarcar) também vale no seu histórico do Simkl.")}>
        <Toggle label={t("Enviar o que eu marco como assistido")} checked={settings.syncHistory} onChange={(v) => onChange({ syncHistory: v })} />
      </Row>
      <Row
        title={t("Trazer o histórico do Simkl")}
        description={`${t("O que você assistiu no Simkl fica marcado como assistido neste perfil (e na conta Nuvio). Roda ao abrir o app, no máximo a cada 6 horas.")}${status?.lastImportMs ? ` ${t("Última vez: {when}.", { when: new Date(status.lastImportMs).toLocaleString(locale()) })}` : ""}`}
      >
        <div className="torbox-key">
          {status?.connected && <button className="ghost" onClick={() => void importNow()} disabled={busy !== null} data-testid="simkl-import">{busy === "import" ? t("Importando…") : t("Importar agora")}</button>}
          <Toggle label={t("Trazer o histórico do Simkl")} checked={settings.importWatched} onChange={(v) => onChange({ importWatched: v })} />
        </div>
      </Row>
    </div>
  );
}
