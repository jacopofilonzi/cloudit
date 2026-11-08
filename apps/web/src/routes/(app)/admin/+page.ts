import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';
import type { PageLoad } from './$types';

// `/admin` has no page of its own: the first tab is the landing page.
export const load: PageLoad = () => {
  redirect(307, resolve('/(app)/admin/users'));
};
