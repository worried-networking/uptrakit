import { writable } from 'svelte/store';

export const isOnline = writable(true); // Default to online

function updateOnlineStatus() {
	isOnline.set(navigator.onLine);
}

// Check initial status
if (typeof window !== 'undefined') {
	updateOnlineStatus(); // Set initial status
	window.addEventListener('online', updateOnlineStatus);
	window.addEventListener('offline', updateOnlineStatus);
}
