<script lang="ts">
  import { Tag } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { STATE_LABEL, STATE_TAG, type DisplayState } from '#lib/status.ts';

  let { state: folderState }: { state: DisplayState } = $props();
</script>

<Tag variant={STATE_TAG[folderState]}>{t(STATE_LABEL[folderState])}</Tag>
