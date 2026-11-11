<script lang="ts">
  // "Check now" for a newer server version (admins): the server checks in the
  // background, the button waits for the result and tells it with a toast. Every
  // instance shares `app.checkingUpdates`, so two buttons never start two checks.
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import { Button, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { app } from '#lib/app.svelte.ts';
  import { errorText } from '#lib/errors.ts';

  let {
    onchecked
  }: { /** After the check ended (whatever its result). */ onchecked?: () => void } = $props();

  async function check() {
    try {
      const result = await app.checkUpdates();
      const summary = app.summary;
      switch (result) {
        case 'newer':
          toast.success(
            t('admin.update_check_newer', { latest: summary?.latest_release?.version ?? '' })
          );
          break;
        case 'up_to_date':
          toast.success(
            t('admin.update_check_up_to_date', { current: summary?.server_version ?? '' })
          );
          break;
        case 'failed':
          toast.error(t('admin.update_check_failed'));
          break;
        case 'slow':
          toast.info(t('admin.update_check_slow'));
          break;
        case 'disabled':
          toast.info(t('admin.update_check_disabled'));
          break;
      }
    } catch (err) {
      toast.error(errorText(err));
    } finally {
      onchecked?.();
    }
  }
</script>

<Button size="sm" loading={app.checkingUpdates} onclick={check}>
  <RefreshCw size={13} aria-hidden="true" />{t('admin.update_check_now')}
</Button>
