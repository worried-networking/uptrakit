import { describe, expect, it } from 'vitest';
import { Actions, hasAction, hasAnyAction, hasActionValue } from './local-types';
import type { User } from './local-types';

function makeUser(actions: readonly string[]): User {
	return {
		id: 'user-1',
		email: 'user@example.com',
		first_name: 'Test',
		last_name: 'User',
		has_pending_email_change: false,
		actions,
		authority: 'ok'
	};
}

describe('hasAction', () => {
	it('returns true when the user holds the action', () => {
		expect(hasAction(makeUser([Actions.HOSTS_READ]), Actions.HOSTS_READ)).toBe(true);
	});

	it('returns false when the user does not hold the action', () => {
		expect(hasAction(makeUser([Actions.HOSTS_READ]), Actions.HOSTS_UPDATE)).toBe(false);
	});

	it('degrades to false for a null/undefined user rather than throwing', () => {
		expect(hasAction(null, Actions.HOSTS_READ)).toBe(false);
		expect(hasAction(undefined, Actions.HOSTS_READ)).toBe(false);
	});
});

describe('hasAnyAction', () => {
	it('returns true when at least one of the given actions is held', () => {
		expect(hasAnyAction(makeUser([Actions.HOSTS_UPDATE]), Actions.HOSTS_READ, Actions.HOSTS_UPDATE)).toBe(true);
	});

	it('returns false when none of the given actions are held', () => {
		expect(hasAnyAction(makeUser([Actions.SOFTWARE_READ]), Actions.HOSTS_READ, Actions.HOSTS_UPDATE)).toBe(false);
	});
});

describe('hasActionValue', () => {
	it('returns true when the user holds the required action', () => {
		expect(hasActionValue(makeUser([Actions.SOFTWARE_READ]), 'software:read')).toBe(true);
	});

	it('returns false when the user does not hold the required action', () => {
		expect(hasActionValue(makeUser([Actions.SOFTWARE_READ]), 'software:update')).toBe(false);
	});

	it('fails open (gates nothing) when the requirement is null, undefined, or empty', () => {
		const user = makeUser([]);
		expect(hasActionValue(user, null)).toBe(true);
		expect(hasActionValue(user, undefined)).toBe(true);
		expect(hasActionValue(user, '')).toBe(true);
	});

	it('degrades to false for a null/undefined user when a requirement is present', () => {
		expect(hasActionValue(null, 'software:read')).toBe(false);
		expect(hasActionValue(undefined, 'software:read')).toBe(false);
	});
});
