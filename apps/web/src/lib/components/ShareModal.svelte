<script lang="ts">
  // Sharing a scope (owner only): add a member by email with a role, change roles,
  // remove members. The server answers `user_not_found` unless an approved, enabled
  // account has that email.
  import X from '@lucide/svelte/icons/x';
  import type { MemberRole, ScopeInfo, ScopeMember } from '@cloudit/api-types';
  import {
    Avatar,
    Banner,
    Button,
    Field,
    Input,
    Modal,
    Select,
    Skeleton,
    confirmAction,
    toast
  } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { addMember, listMembers, removeMember, updateMember } from '#lib/api.ts';
  import { errorText } from '#lib/errors.ts';
  import { isEmail } from '#lib/fields.ts';

  let {
    open = $bindable(false),
    scope,
    onchanged
  }: { open?: boolean; scope: ScopeInfo; onchanged: () => void } = $props();

  let members = $state<ScopeMember[] | null>(null);
  let email = $state('');
  let role = $state<MemberRole>('editor');
  let busy = $state(false);
  let error = $state<string | null>(null);
  let emailError = $state<string | null>(null);

  const roleOptions = $derived<{ value: MemberRole; label: string }[]>([
    { value: 'viewer', label: t('role.viewer') },
    { value: 'editor', label: t('role.editor') },
    { value: 'admin', label: t('role.admin') }
  ]);

  $effect(() => {
    if (!open) return;
    email = '';
    role = 'editor';
    error = null;
    emailError = null;
    members = null;
    void load();
  });

  async function load() {
    try {
      members = await listMembers(scope.id);
    } catch (err) {
      error = errorText(err);
    }
  }

  async function add(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    emailError = null;
    if (!isEmail(email)) {
      emailError = t('field.email_invalid');
      return;
    }
    busy = true;
    try {
      const existing = members?.some((m) => m.user.email === email.trim().toLowerCase());
      const member = await addMember(scope.id, { email, role });
      toast.success(
        existing ? t('share.role_changed') : t('share.added', { name: member.user.display_name })
      );
      email = '';
      await load();
      onchanged();
    } catch (err) {
      emailError = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function changeRole(member: ScopeMember, next: MemberRole) {
    error = null;
    try {
      await updateMember(scope.id, member.user.id, { role: next });
      toast.success(t('share.role_changed'));
      onchanged();
    } catch (err) {
      error = errorText(err);
    }
    await load();
  }

  async function remove(member: ScopeMember) {
    const ok = await confirmAction({
      title: t('share.remove_title', { name: member.user.display_name }),
      message: t('share.remove_body'),
      confirmLabel: t('common.remove'),
      danger: true
    });
    if (!ok) return;
    try {
      await removeMember(scope.id, member.user.id);
      toast.success(t('share.removed', { name: member.user.display_name }));
      onchanged();
    } catch (err) {
      error = errorText(err);
    }
    await load();
  }
</script>

<Modal
  bind:open
  title={t('share.title', { name: scope.name })}
  description={t('share.desc')}
  size="lg"
>
  {#if error}<Banner variant="danger" class="mb-3">{error}</Banner>{/if}

  <form onsubmit={add} class="flex flex-wrap items-start gap-2" novalidate>
    <Field
      label={t('share.email')}
      hint={t('share.email_hint')}
      error={emailError ?? undefined}
      class="min-w-[200px] flex-1"
    >
      {#snippet children(control)}
        <Input {...control} type="email" bind:value={email} autocomplete="off" />
      {/snippet}
    </Field>
    <Field label={t('share.role')}>
      {#snippet children(control)}
        <Select {...control} bind:value={role} options={roleOptions} />
      {/snippet}
    </Field>
    <Button variant="primary" type="submit" loading={busy} class="mt-[22px]"
      >{t('share.add')}</Button
    >
  </form>

  <h3 class="mt-5 mb-2 text-sm font-semibold">{t('share.members')}</h3>
  {#if members === null}
    <Skeleton class="h-10 w-full" />
  {:else}
    <ul class="card divide-y divide-border">
      {#each members as member (member.user.id)}
        <li class="flex items-center gap-3 px-3 py-2.5">
          <Avatar name={member.user.display_name} key={member.user.id} />
          <div class="min-w-0 flex-1">
            <b class="block truncate">{member.user.display_name}</b>
            <span class="block truncate text-xs text-muted">{member.user.email}</span>
          </div>
          {#if member.role === 'owner'}
            <span class="text-xs text-muted">{t('role.owner')}</span>
          {:else}
            <Select
              value={member.role as MemberRole}
              options={roleOptions}
              aria-label={t('share.role_of', { name: member.user.display_name })}
              class="max-w-36"
              onchange={(event) => changeRole(member, event.currentTarget.value as MemberRole)}
            />
            <Button
              variant="ghost"
              size="sm"
              aria-label={t('common.remove')}
              title={t('common.remove')}
              onclick={() => remove(member)}
            >
              <X size={15} aria-hidden="true" />
            </Button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  <h3 class="mt-5 mb-2 text-sm font-semibold">{t('share.roles')}</h3>
  <dl class="note grid gap-1">
    <div>
      <dt class="inline font-semibold text-text">{t('role.viewer')}:</dt>
      <dd class="inline">{t('role.viewer_desc')}</dd>
    </div>
    <div>
      <dt class="inline font-semibold text-text">{t('role.editor')}:</dt>
      <dd class="inline">{t('role.editor_desc')}</dd>
    </div>
    <div>
      <dt class="inline font-semibold text-text">{t('role.admin')}:</dt>
      <dd class="inline">{t('role.admin_desc')}</dd>
    </div>
  </dl>
</Modal>
