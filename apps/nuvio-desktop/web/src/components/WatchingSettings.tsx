// Settings → Reprodução: when something counts as watched, skip buttons and the next-episode card
// (the original's "Pular intro" and t("Próximo episódio") settings, plus the watched percentage).

import { useEffect, useState } from "react";
import type { PlayerSettings } from "../types";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import { decimal, t } from "../i18n";

const AUTO_KINDS: [string, string][] = [
  ["intro", "Introdução"],
  ["recap", "Recapitulação"],
  ["outro", "Encerramento"],
  ["movie-credits", "Créditos de filmes"],
];

function Range({ label, value, min, max, step, unit, onChange }: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange(v: number): void }) {
  return (
    <span className="slider">
      <input type="range" min={min} max={max} step={step} value={value} aria-label={label} onChange={(e) => onChange(Number(e.target.value))} />
      <output>{decimal(value)}{unit}</output>
    </span>
  );
}

export function WatchingSettings({ value: p, onChange }: { value: PlayerSettings; onChange(change: (p: PlayerSettings) => PlayerSettings): void }) {
  const percentMode = p.nextEpisodeThresholdMode !== "MINUTES_BEFORE_END";
  const [clientId, setClientId] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [keyNote, setKeyNote] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api.skipCredentials().then((keys) => {
      if (cancelled) return;
      setClientId(keys.animeSkipClientId);
      setApiKey(keys.introDbApiKey);
    }, () => {});
    return () => { cancelled = true; };
  }, []);

  const saveKeys = async () => {
    setKeyNote(null);
    try {
      await api.skipCredentialsSet(clientId, apiKey);
      setKeyNote(t("Chaves salvas."));
    } catch (e) {
      setKeyNote(errorText(e));
    }
  };
  return (
    <>
      <h3 className="settings-subhead">{t("Assistido e próximo episódio")}</h3>
      <Row title={t("Considerar assistido a partir de")} description={t("Antes disso o episódio ou filme fica em Continuar assistindo. Depois, a série segue com o próximo episódio. Vale só neste computador: os outros apps Nuvio usam sempre 90%.")}>
        <Range label={t("Considerar assistido a partir de")} value={p.watchedPercent} min={90} max={100} step={1} unit="%" onChange={(v) => onChange((x) => ({ ...x, watchedPercent: v }))} />
      </Row>
      <Row title={t("Mostrar o próximo episódio")} description={t("Quando o cartão aparece no player. Com o tempo do encerramento conhecido, ele aparece junto com os créditos finais.")}>
        <select aria-label={t("Mostrar o próximo episódio")} value={p.nextEpisodeThresholdMode} onChange={(e) => onChange((x) => ({ ...x, nextEpisodeThresholdMode: e.target.value as PlayerSettings["nextEpisodeThresholdMode"] }))}>
          <option value="PERCENTAGE">{t("Por porcentagem")}</option>
          <option value="MINUTES_BEFORE_END">{t("Por minutos antes do fim")}</option>
        </select>
      </Row>
      {percentMode ? (
        <Row title={t("Porcentagem do episódio")}>
          <Range label={t("Porcentagem do episódio")} value={p.nextEpisodeThresholdPercent} min={97} max={100} step={0.5} unit="%" onChange={(v) => onChange((x) => ({ ...x, nextEpisodeThresholdPercent: v }))} />
        </Row>
      ) : (
        <Row title={t("Minutos antes do fim")}>
          <Range label={t("Minutos antes do fim")} value={p.nextEpisodeThresholdMinutes} min={0} max={3.5} step={0.5} unit=" min" onChange={(v) => onChange((x) => ({ ...x, nextEpisodeThresholdMinutes: v }))} />
        </Row>
      )}

      <h3 className="settings-subhead">{t("Próximo episódio automático")}</h3>
      <Row title={t("Reproduzir o próximo episódio automaticamente")} description={t("Escolhe uma fonte e começa o próximo episódio quando o atual termina.")}>
        <Toggle label={t("Reproduzir o próximo episódio automaticamente")} checked={p.autoPlayNextEpisodeEnabled} onChange={(v) => onChange((x) => ({ ...x, autoPlayNextEpisodeEnabled: v }))} />
      </Row>
      <Row title={t("Preferir o mesmo grupo de fontes")} description={t("No próximo episódio, tenta primeiro o addon da fonte atual.")}>
        <Toggle label={t("Preferir o mesmo grupo de fontes")} checked={p.autoPlayPreferBingeGroup} onChange={(v) => onChange((x) => ({ ...x, autoPlayPreferBingeGroup: v }))} />
      </Row>
      {p.autoPlayNextEpisodeEnabled && (
        <>
          <Row title={t("Permitir outra fonte se a preferida falhar")}>
            <Toggle label={t("Permitir outra fonte se a preferida falhar")} checked={p.autoPlayNextEpisodeFallbackEnabled} onChange={(v) => onChange((x) => ({ ...x, autoPlayNextEpisodeFallbackEnabled: v }))} />
          </Row>
          <Row title={t("Pré-carregar fontes do próximo episódio")} description={t("Busca as fontes antes do fim do episódio atual.")}>
            <Toggle label={t("Pré-carregar fontes do próximo episódio")} checked={p.preloadNextEpisodeSources} onChange={(v) => onChange((x) => ({ ...x, preloadNextEpisodeSources: v }))} />
          </Row>
        </>
      )}

      <h3 className="settings-subhead">{t("Player")}</h3>
      <Row title={t("Títulos dos capítulos")} description={t("Mostra o nome do capítulo quando o vídeo os traz.")}>
        <Toggle label={t("Títulos dos capítulos")} checked={p.chapterTitleEnabled} onChange={(v) => onChange((x) => ({ ...x, chapterTitleEnabled: v }))} />
      </Row>
      <Row title={t("Segurar para acelerar")} description={t("Segure o botão de avanço no player para reproduzir mais rápido.")}>
        <Toggle label={t("Segurar para acelerar")} checked={p.holdToSpeedEnabled} onChange={(v) => onChange((x) => ({ ...x, holdToSpeedEnabled: v }))} />
      </Row>
      {p.holdToSpeedEnabled && (
        <Row title={t("Velocidade ao segurar")}>
          <select aria-label={t("Velocidade ao segurar")} value={p.holdToSpeedValue} onChange={(e) => onChange((x) => ({ ...x, holdToSpeedValue: Number(e.target.value) }))}>
            {[1, 1.5, 2, 2.5, 3, 3.5, 4].map((speed) => <option key={speed} value={speed}>{speed}×</option>)}
          </select>
        </Row>
      )}
      <Row title={t("Avisos de conteúdo")} description={t("Mostra o guia parental quando disponível para o título.")}>
        <Toggle label={t("Avisos de conteúdo")} checked={p.showParentalGuide} onChange={(v) => onChange((x) => ({ ...x, showParentalGuide: v }))} />
      </Row>

      <h3 className="settings-subhead">{t("Áudio e legendas avançados")}</h3>
      <Row title={t("Passagem de áudio (passthrough)")} description={t("Envia AC3, E-AC3, DTS, DTS-HD e TrueHD diretos para o recetor. Se o equipamento não aceitar, o mpv volta a PCM.")}>
        <Toggle label={t("Passagem de áudio (passthrough)")} checked={p.audioPassthrough} onChange={(v) => onChange((x) => ({ ...x, audioPassthrough: v }))} />
      </Row>
      <Row title={t("Página de código das legendas")} description={t("Para ficheiros SRT antigos sem BOM. Para árabe, escolha cp1256.")}>
        <select aria-label={t("Página de código das legendas")} value={p.subtitleCodepage} onChange={(e) => onChange((x) => ({ ...x, subtitleCodepage: e.target.value }))}>
          {[
            ["", t("Automático")],
            ["UTF-8", "UTF-8"],
            ["cp1252", "cp1252 · Latin-1"],
            ["cp1256", "cp1256 · Árabe"],
            ["cp1251", "cp1251 · Cirílico"],
            ["cp1254", "cp1254 · Turco"],
            ["iso-8859-6", "iso-8859-6 · Árabe"],
            ["gbk", "GBK · Chinês"],
          ].map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select>
      </Row>

      <h3 className="settings-subhead">{t("Pular introdução e créditos")}</h3>
      <Row title={t("Botões de pular")} description={t("Os tempos vêm dos capítulos do vídeo, do IntroDB e, para animes, do AniSkip. Para isso o app envia a esses serviços o código público do título e o número do episódio.")}>
        <Toggle label={t("Botões de pular")} checked={p.skipSegmentsEnabled} onChange={(v) => onChange((x) => ({ ...x, skipSegmentsEnabled: v }))} />
      </Row>
      {p.skipSegmentsEnabled &&
        AUTO_KINDS.map(([kind, name]) => (
          <Row key={kind} title={t("Pular automaticamente: {name}", { name: t(name) })}>
            <Toggle
              label={t("Pular automaticamente: {name}", { name: t(name) })}
              checked={p.autoSkipSegments.includes(kind)}
              onChange={(v) => onChange((x) => ({ ...x, autoSkipSegments: [...x.autoSkipSegments.filter((k) => k !== kind), ...(v ? [kind] : [])].sort() }))}
            />
          </Row>
        ))}
      <Row title={t("Anime-Skip")} description={t("Serviço extra para animes: usa o seu Client ID e o AniList do título. Sem mapeamento, simplesmente não oferece tempos.")}>
        <Toggle label={t("Anime-Skip")} checked={p.animeSkipEnabled} onChange={(v) => onChange((x) => ({ ...x, animeSkipEnabled: v }))} />
      </Row>
      <Row title={t("Enviar tempos à IntroDB")} description={t("Mostra um botão no player para enviar o tempo de uma abertura, recapitulação ou encerramento que o app encontrou.")}>
        <Toggle label={t("Enviar tempos")} checked={p.introSubmitEnabled} onChange={(v) => onChange((x) => ({ ...x, introSubmitEnabled: v }))} />
      </Row>
      <Row title={t("ID de cliente do Anime-Skip")} description={t("Pegue o seu ID em anime-skip.com. A chave fica salva no app e sincronizada com a conta.")}>
        <input type="text" value={clientId} aria-label={t("ID de cliente do Anime-Skip")} onChange={(e) => setClientId(e.target.value)} />
      </Row>
      <Row title={t("Chave da API da IntroDB")} description={t("Necessária para enviar tempos. A chave é verificada ao salvar.")}>
        <input type="password" value={apiKey} aria-label={t("Chave da API da IntroDB")} onChange={(e) => setApiKey(e.target.value)} />
        <button className="ghost" onClick={() => void saveKeys()}>{t("Salvar chaves")}</button>
      </Row>
      {keyNote && <p className="muted small" role="status">{keyNote}</p>}
    </>
  );
}
