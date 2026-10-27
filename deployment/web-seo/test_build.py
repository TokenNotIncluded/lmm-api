"""Regression for a release catalog that cannot serve an overlay public page."""
import unittest

from build import route_patterns


class PublicRouteProvenanceTests(unittest.TestCase):
    routes = [
        '/', '/guide/', '/security/', '/privacy-policy', '/user-agreement',
        '/refund-policy', '/dashboard/', '/sign-in', '/keys/', '/pricing/',
    ]

    def patterns(self, routes):
        catalog = '  fullPaths:\n' + ''.join(f"    | '{route}'\n" for route in routes)
        catalog += '  fileRoutesByTo:\n'
        bundle = ';'.join(
            f'n{i}=r.update({{id:"{i}",path:"{route}",getParentRoute:()=>root}})'
            for i, route in enumerate(routes)
        )
        return route_patterns(catalog, bundle)

    def test_public_pages_allow_the_generated_trailing_slash(self):
        self.assertEqual(len(self.patterns(self.routes)), len(self.routes) + 2)

    def test_public_page_must_exist_even_when_bundle_and_catalog_agree(self):
        routes = ['/help/' if route == '/guide/' else route for route in self.routes]
        with self.assertRaisesRegex(ValueError, r'Public overlay paths.*\/guide'):
            self.patterns(routes)


if __name__ == '__main__':
    unittest.main()
