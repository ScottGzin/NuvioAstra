// Settings → Integrações → Discord: Rich Presence on this computer (off by default, like the
// original). Discord must be open on this computer; nothing else is needed.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import { t } from "../i18n";

export function DiscordSection() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.discordEnabled().then(setEnabled, (e) => setError(errorText(e)));
  }, []);

  const change = async (v: boolean) => {
    setError(null);
    try {
      await api.discordSetEnabled(v);
      setEnabled(v);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="settings-group" data-testid="discord-settings">
      <p className="muted">{t("Mostra no seu perfil do Discord o que você está assistindo, com o episódio, o pôster e o tempo restante. Precisa do Discord aberto neste computador.")}</p>
      <Row title={t("Mostrar no Discord o que estou assistindo")} description={t("Vale para este computador.")}>
        {enabled !== null && <Toggle label={t("Mostrar no Discord o que estou assistindo")} checked={enabled} onChange={(v) => void change(v)} />}
      </Row>
      {error && <div className="row-error" role="alert">{t(error)}</div>}
    </div>
  );
}
