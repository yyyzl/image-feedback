"""Original synthetic large-image input for memory testing; not an app screenshot."""
from pathlib import Path
from PIL import Image
root=Path(__file__).resolve().parents[1]
(root/'artifacts').mkdir(exist_ok=True)
with Image.open(root/'assets'/'demo.png') as image:
    image.resize((6000,4000),Image.Resampling.LANCZOS).save(root/'artifacts'/'perf-large.png')
print('Created a 6000 x 4000 PNG test input (24 megapixels).')
