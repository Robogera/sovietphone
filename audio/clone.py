from TTS.api import TTS

tts = TTS("tts_models/multilingual/multi-dataset/xtts_v2")

tts.tts_to_file(
    text="Звать меня пенисова Галина Михайловна. Моё любимое число - Шестнадцать. Номер шестнадцать. Мой номер - шестнадцать.",
    speaker_wav="ref.wav",
    language="ru",
    file_path="16.wav",
)
