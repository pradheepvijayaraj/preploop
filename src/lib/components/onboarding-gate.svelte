<script lang="ts">
  import type { Snippet } from "svelte";
  import WelcomePage from "$lib/components/welcome-page.svelte";
  import { getSettings } from "$lib/stores/settings.svelte";

  let { children } = $props<{ children: Snippet }>();
  const completed = $derived(getSettings().hasCompletedOnboarding);
</script>

{#if completed}
  {@render children()}
{:else}
  <WelcomePage />
{/if}
