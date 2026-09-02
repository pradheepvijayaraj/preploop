<script lang="ts">
  import { Checkbox } from "$lib/components/ui/checkbox";
  import { Label } from "$lib/components/ui/label";
  import { Switch } from "$lib/components/ui/switch";

  let switchChecked = $state(false);
  let checkboxChecked = $state(false);
</script>

<div>
  <Label for="primitive-switch">Immediate feedback</Label>
  <Switch id="primitive-switch" bind:checked={switchChecked} />
</div>

<div>
  <Label for="primitive-checkbox">Select answer</Label>
  <Checkbox id="primitive-checkbox" bind:checked={checkboxChecked} />
</div>

<output data-testid="primitive-state">
  {switchChecked ? "switch-on" : "switch-off"}
  {checkboxChecked ? "checkbox-on" : "checkbox-off"}
</output>
