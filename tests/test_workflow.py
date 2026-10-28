import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from scribiva import engine

class WorkflowTests(unittest.TestCase):
    def test_toggle_start_and_stop_dispatches_worker(self):
        with tempfile.TemporaryDirectory() as d:
            session=Path(d)/'session.json'
            lock=Path(d)/'lock'
            audio=Path(d)/'record.wav'
            audio.write_bytes(b'WAV')
            with patch.object(engine,'STATE_PATH',session),patch.object(engine,'LOCK_PATH',lock), \
                 patch.object(engine,'secure_dirs'), patch.object(engine,'init_logging'), \
                 patch.object(engine,'notify'), patch.object(engine,'read_config',return_value={'max_record_seconds':180}), \
                 patch.object(engine,'mic_node',return_value='alsa_input.fake'), \
                 patch.object(engine.audio,'start',return_value=(1234,audio)), \
                 patch.object(engine.audio,'stop'), patch.object(engine,'_belongs_here',return_value=True), \
                 patch.object(engine,'launch_aux') as launch, patch.object(engine.logger,'info'):
                self.assertEqual(engine.toggle(),'recording')
                self.assertEqual(engine.state()['phase'],'recording')
                self.assertEqual(launch.call_args.args[0],'watchdog')
                with patch.object(engine.time, 'time', return_value=engine.state()['started']+3):
                    self.assertEqual(engine.toggle(),'processing')
                self.assertEqual(engine.state()['phase'],'processing')
                self.assertEqual(launch.call_args.args[0],'worker')

if __name__ == '__main__':
    unittest.main()
