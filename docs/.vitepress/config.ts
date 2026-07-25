import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

export default withMermaid(
  defineConfig({
    title: 'nanites',
    description: 'Turn-primitive orchestration framework',
    base: '/nanites/',
    srcExclude: ['**/CLAUDE.md'],
    themeConfig: {
      nav: [
        { text: 'Guide', link: '/guide/' },
        { text: 'rhi', link: 'https://docs.rhi.zone/' },
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
      search: {
        provider: 'local'
      },
      editLink: {
        pattern: 'https://github.com/rhi-zone/nanites/edit/master/docs/:path',
        text: 'Edit this page on GitHub'
      },
    },
  }),
)
