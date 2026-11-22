import { componentsA } from "./pt-BR/components-a";
import { componentsB } from "./pt-BR/components-b";
import { core } from "./pt-BR/core";
import { screensA } from "./pt-BR/screens-a";
import { screensB } from "./pt-BR/screens-b";

export const ptBR: Record<string, string> = { ...core, ...screensA, ...screensB, ...componentsA, ...componentsB };
