(function () {
	var m = localStorage.getItem('theme-mode') || 'system';
	var d = m === 'dark' || (m === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
	if (d) document.documentElement.classList.add('dark');
})();
