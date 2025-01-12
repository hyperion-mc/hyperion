import {defineConfig} from 'vitepress'

import {withMermaid} from 'vitepress-plugin-mermaid';

import footnote from 'markdown-it-footnote'

// https://vitepress.dev/reference/site-config
const config = defineConfig({
    title: "Hyperion",
    description: "The most advanced Minecraft game engine built in Rust",
    base: "/hyperion/",
    markdown: {
        math: true,
        config: (md) => {
            md.use(footnote)
        }
    },
    themeConfig: {
        // https://vitepress.dev/reference/default-theme-config
        nav: [
            {text: 'Home', link: '/'},
            {text: 'Architecture', link: '/architecture/introduction'},
            {text: 'Tag', link: '/tag/introduction'},
        ],

        sidebar: [
            {
                text: 'Architecture',
                items: [
                    {text: 'Introduction', link: '/architecture/introduction'},
                    {text: 'Game Server', link: '/architecture/game-server'},
                    {text: 'Proxy', link: '/architecture/proxy'},
                ]
            },
            {
                text: 'Tag',
                items: [
                    {text: '10,000 Player PvP', link: '/tag/introduction'},
                ]
            }
        ],
        socialLinks: [
            {icon: 'github', link: 'https://github.com/hyperion-mc/hyperion'}
        ]
    }
})


export default withMermaid(config);