<script lang="ts">
  // Background jobs: what ran last, and "run now". While a run is requested or running
  // the list refreshes every few seconds, then stops.
  import { onMount } from 'svelte';
  import Play from '@lucide/svelte/icons/play';
  import type { JobInfo, JobName, JobOutcome, JobRunInfo } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Card,
    Skeleton,
    Spinner,
    Tag,
    formatBytes,
    formatDateTime,
    formatNumber,
    formatRelative,
    toast,
    type TagVariant
  } from '@cloudit/ui';
  import { getLang, t, type MessageKey } from '@cloudit/ui/i18n';
  import { listJobs, runJob } from '#lib/api.ts';
  import { errorText } from '#lib/errors.ts';

  const POLL_MS = 3000;

  let jobs = $state<JobInfo[] | null>(null);
  let error = $state<string | null>(null);
  let starting = $state<JobName | null>(null);

  const lang = $derived(getLang());

  async function load() {
    try {
      jobs = await listJobs();
      error = null;
    } catch (err) {
      error = errorText(err);
    }
  }

  onMount(() => void load());

  const isRunning = (job: JobInfo) => job.recent_runs[0]?.outcome === 'running';
  const isPending = (job: JobInfo) => job.requested_at !== null || isRunning(job);
  const anyPending = $derived(jobs?.some(isPending) ?? false);

  // Poll only while something is requested or running.
  $effect(() => {
    if (!anyPending) return;
    const timer = setInterval(() => void load(), POLL_MS);
    return () => clearInterval(timer);
  });

  async function run(job: JobInfo) {
    starting = job.name;
    try {
      const updated = await runJob(job.name);
      jobs = jobs?.map((j) => (j.name === updated.name ? updated : j)) ?? null;
      toast.success(t('admin.job_requested'));
    } catch (err) {
      toast.error(errorText(err));
    } finally {
      starting = null;
    }
  }

  const OUTCOME: Record<JobOutcome, { key: MessageKey; variant: TagVariant }> = {
    running: { key: 'admin.job_outcome_running', variant: 'primary' },
    succeeded: { key: 'admin.job_outcome_succeeded', variant: 'success' },
    failed: { key: 'admin.job_outcome_failed', variant: 'danger' },
    interrupted: { key: 'admin.job_outcome_interrupted', variant: 'warning' }
  };

  const NAMES: Record<JobName, { title: MessageKey; desc: MessageKey }> = {
    retention: { title: 'admin.job_retention', desc: 'admin.job_retention_desc' },
    gc: { title: 'admin.job_gc', desc: 'admin.job_gc_desc' },
    orphan_sweep: { title: 'admin.job_orphan_sweep', desc: 'admin.job_orphan_sweep_desc' },
    trash_purge: { title: 'admin.job_trash_purge', desc: 'admin.job_trash_purge_desc' },
    cleanup: { title: 'admin.job_cleanup', desc: 'admin.job_cleanup_desc' },
    update_check: { title: 'admin.job_update_check', desc: 'admin.job_update_check_desc' }
  };

  const COUNT_KEYS: Record<string, MessageKey> = {
    sessions: 'admin.count_sessions',
    device_auth_codes: 'admin.count_device_auth_codes',
    oauth_flows: 'admin.count_oauth_flows',
    pending_signups: 'admin.count_pending_signups',
    password_resets: 'admin.count_password_resets',
    invites: 'admin.count_invites',
    job_runs: 'admin.count_job_runs',
    objects_deleted: 'admin.count_objects_deleted',
    chunks_deleted: 'admin.count_chunks_deleted',
    manifests_deleted: 'admin.count_manifests_deleted',
    bytes_freed: 'admin.count_bytes_freed',
    batches_skipped: 'admin.count_batches_skipped',
    objects_listed: 'admin.count_objects_listed',
    unrecognized: 'admin.count_unrecognized',
    folders: 'admin.count_folders',
    snapshots_deleted: 'admin.count_snapshots_deleted',
    scopes_deleted: 'admin.count_scopes_deleted',
    folders_deleted: 'admin.count_folders_deleted',
    checked: 'admin.count_checked',
    newer: 'admin.count_newer',
    no_release: 'admin.count_no_release',
    disabled: 'admin.count_disabled'
  };

  function counts(run: JobRunInfo): string[] {
    return Object.entries(run.counts)
      .filter(([, value]) => value > 0)
      .map(([key, value]) => {
        const label = COUNT_KEYS[key] ? t(COUNT_KEYS[key]) : key.replaceAll('_', ' ');
        return `${label}: ${key === 'bytes_freed' ? formatBytes(value, lang) : formatNumber(value, lang)}`;
      });
  }

  /** "every 15 minutes", "every day": the interval in the largest whole unit. */
  function every(seconds: number): string {
    if (seconds % 86400 === 0) return t('admin.every_days', { n: seconds / 86400 });
    if (seconds % 3600 === 0) return t('admin.every_hours', { n: seconds / 3600 });
    return t('admin.every_minutes', { n: Math.round(seconds / 60) });
  }
</script>

<svelte:head>
  <title>{t('admin.tab_maintenance')} · {t('admin.title')} · {t('app.name')}</title>
</svelte:head>

<p class="mb-4 max-w-[70ch] text-muted">{t('admin.maintenance_desc')}</p>

{#if error}
  <Banner variant="danger">
    {error}
    {#snippet actions()}<Button size="sm" onclick={load}>{t('common.retry')}</Button>{/snippet}
  </Banner>
{:else if jobs === null}
  <Skeleton class="h-64 w-full" />
{:else}
  <ul class="flex flex-col gap-3" aria-label={t('admin.tab_maintenance')}>
    {#each jobs as job (job.name)}
      {@const last = job.recent_runs[0]}
      {@const busy = isPending(job)}
      <li data-job={job.name}>
        <Card class="flex flex-wrap items-start gap-4">
          <div class="min-w-0 flex-1 basis-72">
            <div class="flex flex-wrap items-center gap-2">
              <h3>{t(NAMES[job.name].title)}</h3>
              <span class="text-xs text-muted">{every(job.interval_secs)}</span>
              {#if last}
                {@const outcome = OUTCOME[last.outcome]}
                <Tag variant={outcome.variant}>{t(outcome.key)}</Tag>
              {/if}
            </div>
            <p class="text-xs text-muted">{t(NAMES[job.name].desc)}</p>
            <p class="mt-2 text-xs" data-last-run>
              {#if last}
                <span class="text-muted">{t('admin.job_last_run')}</span>
                <span title={formatDateTime(last.started_at, lang)}>
                  {formatRelative(last.started_at, lang)}
                </span>
                {#if counts(last).length > 0}
                  <span class="text-muted"> · {counts(last).join(' · ')}</span>
                {/if}
              {:else}
                <span class="text-muted">{t('admin.job_never_ran')}</span>
              {/if}
            </p>
            {#if last?.error}
              <p class="mt-1 rounded-md bg-danger-soft px-2 py-1 font-mono text-xs break-words">
                {last.error}
              </p>
            {/if}
          </div>
          <div class="flex flex-none items-center gap-2">
            {#if busy}
              <span class="flex items-center gap-1.5 text-xs text-muted" role="status">
                <Spinner size={14} />
                {job.requested_at && !isRunning(job)
                  ? t('admin.job_queued')
                  : t('admin.job_outcome_running')}
              </span>
            {/if}
            <Button
              size="sm"
              disabled={busy}
              loading={starting === job.name}
              onclick={() => run(job)}
            >
              <Play size={13} aria-hidden="true" />{t('admin.job_run')}
            </Button>
          </div>
        </Card>
      </li>
    {/each}
  </ul>
{/if}
