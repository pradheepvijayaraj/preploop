import { writable } from "svelte/store";

// The updater survives route changes; only the catalog home exposes its button.
export const updaterHome = writable(false);
