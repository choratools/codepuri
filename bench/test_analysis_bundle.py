import unittest
from analysis_bundle import components, java_status


class AnalysisBundleTests(unittest.TestCase):
    def test_emitted_body_with_failure_receipt_is_still_readable_body(self):
        evidence = [{'status': 'body', 'complete': False, 'dependency': True}, {'status': 'unavailable', 'reason': 'closure'}]
        self.assertEqual(java_status(evidence), 'body_with_diagnostic')

    def test_missing_body_is_not_upgraded_and_no_record_is_unknown(self):
        self.assertEqual(java_status([{'status': 'unavailable'}]), 'unavailable')
        self.assertEqual(java_status([]), 'not_observed')
        self.assertEqual(java_status([{'status': 'declaration'}]), 'declaration')

    def test_component_permissions_filters_and_provider_attributes_keep_scope(self):
        text = '''E: manifest
  E: application
    A: android:name(0x1)="p.App"
    E: activity
      A: android:name(0x1)="p.Main"
      A: android:exported(0x2)=(type 0x12)0xffffffff
      E: intent-filter
        E: action
          A: android:name(0x1)="android.intent.action.VIEW"
        E: data
          A: android:scheme(0x3)="fixture"
    E: provider
      A: android:name(0x1)="p.Provider"
      A: android:permission(0x4)="p.PRIVATE"
      A: android:authorities(0x5)="p.files"
'''
        app, activity, provider = components(text)
        self.assertEqual(app['name'], 'p.App')
        self.assertEqual(activity['actions'], ['android.intent.action.VIEW'])
        self.assertEqual(activity['data'], [{'scheme': 'fixture'}])
        self.assertEqual(provider['permission'], 'p.PRIVATE')
        self.assertEqual(provider['attributes']['authorities'], 'p.files')
        self.assertEqual(provider['actions'], [])

    def test_separate_intent_filters_do_not_create_false_uri_combinations(self):
        text = '''E: manifest
  E: application
    E: activity
      A: android:name(0x1)="p.Router"
      E: intent-filter
        E: data
          A: android:scheme(0x2)="https"
          A: android:host(0x3)="example.test"
      E: intent-filter
        E: data
          A: android:scheme(0x2)="custom"
          A: android:host(0x3)="app"
'''
        activity = components(text)[1]
        self.assertEqual(activity['filters'][0]['data'], [{'scheme': 'https', 'host': 'example.test'}])
        self.assertEqual(activity['filters'][1]['data'], [{'scheme': 'custom', 'host': 'app'}])


if __name__ == '__main__':
    unittest.main()
