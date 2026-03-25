import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'nanites',
  description: 'Turn-primitive orchestration framework',
  base: '/nanites/',
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/' },
      { text: 'rhi', link: 'https://rhi.zone/' },
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/' },
        ],
      },
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/rhi-zone/nanites' },
    ],
  },
})
