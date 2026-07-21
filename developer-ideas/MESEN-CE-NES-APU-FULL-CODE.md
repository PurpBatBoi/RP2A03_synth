"ApuEnvelope.h"
```cpp
#pragma once
#include "pch.h"
#include "NES/APU/ApuLengthCounter.h"
#include "NES/NesConsole.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"

class ApuEnvelope : public ISerializable
{
private:
	bool _constantVolume = false;
	uint8_t _volume = 0;

	bool _start = false;
	int8_t _divider = 0;
	uint8_t _counter = 0;

public:
	ApuLengthCounter LengthCounter;

	ApuEnvelope(AudioChannel channel, NesConsole* console) : LengthCounter(channel, console)
	{
	}

	void InitializeEnvelope(uint8_t regValue)
	{
		LengthCounter.InitializeLengthCounter((regValue & 0x20) == 0x20);
		_constantVolume = (regValue & 0x10) == 0x10;
		_volume = regValue & 0x0F;
	}

	void ResetEnvelope()
	{
		_start = true;
	}

	uint32_t GetVolume()
	{
		if(LengthCounter.GetStatus()) {
			if(_constantVolume) {
				return _volume;
			} else {
				return _counter;
			}
		} else {
			return 0;
		}
	}

	void Reset(bool softReset)
	{
		LengthCounter.Reset(softReset);
		_constantVolume = false;
		_volume = 0;
		_start = false;
		_divider = 0;
		_counter = 0;
	}

	void Serialize(Serializer& s) override
	{
		SV(_constantVolume);
		SV(_volume);
		SV(_start);
		SV(_divider);
		SV(_counter);
		SV(LengthCounter);
	}

	void TickEnvelope()
	{
		if(!_start) {
			_divider--;
			if(_divider < 0) {
				_divider = _volume;
				if(_counter > 0) {
					_counter--;
				} else if(LengthCounter.IsHalted()) {
					_counter = 15;
				}
			}
		} else {
			_start = false;
			_counter = 15;
			_divider = _volume;
		}
	}

	ApuEnvelopeState GetState()
	{
		ApuEnvelopeState state;
		state.ConstantVolume = _constantVolume;
		state.Counter = _counter;
		state.Divider = _divider;
		state.Loop = LengthCounter.IsHalted();
		state.StartFlag = _start;
		state.Volume = _volume;
		return state;
	}
};
```


"ApuFrameCounter"
```cpp
#pragma once
#include "pch.h"
#include "NES/INesMemoryHandler.h"
#include "NES/NesConsole.h"
#include "NES/NesCpu.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"

enum class FrameType
{
	None = 0,
	QuarterFrame = 1,
	HalfFrame = 2,
};

class ApuFrameCounter : public INesMemoryHandler, public ISerializable
{
private:
	const int32_t _stepCyclesNtsc[2][6] = {
		{ 7457, 14913, 22371, 29828, 29829, 29830 },
		{ 7457, 14913, 22371, 29829, 37281, 37282 }
	};
	const int32_t _stepCyclesPal[2][6] = {
		{ 8313, 16627, 24939, 33252, 33253, 33254 },
		{ 8313, 16627, 24939, 33253, 41565, 41566 }
	};
	const FrameType _frameType[2][6] = { { FrameType::QuarterFrame, FrameType::HalfFrame, FrameType::QuarterFrame, FrameType::None, FrameType::HalfFrame, FrameType::None },
		{ FrameType::QuarterFrame, FrameType::HalfFrame, FrameType::QuarterFrame, FrameType::None, FrameType::HalfFrame, FrameType::None } };

	NesConsole* _console = nullptr;
	int32_t _stepCycles[2][6] = {};
	int32_t _previousCycle = 0;
	uint32_t _currentStep = 0;
	uint32_t _stepMode = 0; //0: 4-step mode, 1: 5-step mode
	bool _inhibitIRQ = false;
	uint8_t _blockFrameCounterTick = 0;
	int16_t _newValue = 0;
	int8_t _writeDelayCounter = 0;

	bool _irqFlag = false;
	uint64_t _irqFlagClearClock = 0;

public:
	ApuFrameCounter(NesConsole* console)
	{
		_console = console;
		Reset(false);
	}

	void Reset(bool softReset)
	{
		_previousCycle = 0;
		_irqFlag = false;
		_irqFlagClearClock = 0;

		//"After reset: APU mode in $4017 was unchanged", so we need to keep whatever value _stepMode has for soft resets
		if(!softReset) {
			_stepMode = 0;
		}

		_currentStep = 0;

		//"After reset or power-up, APU acts as if $4017 were written with $00 from 9 to 12 clocks before first instruction begins."
		//This is emulated in the CPU::Reset function
		//Reset acts as if $00 was written to $4017
		_newValue = _stepMode ? 0x80 : 0x00;
		_writeDelayCounter = 3;
		_inhibitIRQ = false;

		_blockFrameCounterTick = 0;
	}

	void Serialize(Serializer& s) override
	{
		SV(_previousCycle);
		SV(_currentStep);
		SV(_stepMode);
		SV(_inhibitIRQ);
		SV(_blockFrameCounterTick);
		SV(_writeDelayCounter);
		SV(_newValue);
		SV(_irqFlag);
		SV(_irqFlagClearClock);

		if(!s.IsSaving()) {
			SetRegion(_console->GetRegion());
		}
	}

	void SetRegion(ConsoleRegion region)
	{
		switch(region) {
			case ConsoleRegion::Auto:
				//Auto should never be set here
				break;

			case ConsoleRegion::Ntsc:
			case ConsoleRegion::Dendy:
				memcpy(_stepCycles, _stepCyclesNtsc, sizeof(_stepCycles));
				break;

			case ConsoleRegion::Pal:
				memcpy(_stepCycles, _stepCyclesPal, sizeof(_stepCycles));
				break;
		}
	}

	uint32_t Run(int32_t& cyclesToRun)
	{
		uint32_t cyclesRan;

		if(_previousCycle + cyclesToRun >= _stepCycles[_stepMode][_currentStep]) {
			if(_stepMode == 0 && _currentStep >= 3) {
				//Set irq on the last 3 cycles for 4-step mode
				_irqFlag = true;
				_irqFlagClearClock = 0;

				if(!_inhibitIRQ) {
					_console->GetCpu()->SetIrqSource(IRQSource::FrameCounter);
				} else if(_currentStep == 5) {
					_irqFlag = false;
					_irqFlagClearClock = 0;
				}
			}

			FrameType type = _frameType[_stepMode][_currentStep];
			if(type != FrameType::None && !_blockFrameCounterTick) {
				_console->GetApu()->FrameCounterTick(type);

				//Do not allow writes to 4017 to clock the frame counter for the next cycle (i.e this odd cycle + the following even cycle)
				_blockFrameCounterTick = 2;
			}

			if(_stepCycles[_stepMode][_currentStep] < _previousCycle) {
				//This can happen when switching from PAL to NTSC, which can cause a freeze (endless loop in APU)
				cyclesRan = 0;
			} else {
				cyclesRan = _stepCycles[_stepMode][_currentStep] - _previousCycle;
			}

			cyclesToRun -= cyclesRan;

			_currentStep++;
			if(_currentStep == 6) {
				_currentStep = 0;
				_previousCycle = 0;
			} else {
				_previousCycle += cyclesRan;
			}
		} else {
			cyclesRan = cyclesToRun;
			cyclesToRun = 0;
			_previousCycle += cyclesRan;
		}

		if(_newValue >= 0) {
			_writeDelayCounter--;
			if(_writeDelayCounter == 0) {
				//Apply new value after the appropriate number of cycles has elapsed
				_stepMode = ((_newValue & 0x80) == 0x80) ? 1 : 0;

				_writeDelayCounter = -1;
				_currentStep = 0;
				_previousCycle = 0;
				_newValue = -1;

				if(_stepMode && !_blockFrameCounterTick) {
					//"Writing to $4017 with bit 7 set will immediately generate a clock for both the quarter frame and the half frame units, regardless of what the sequencer is doing."
					_console->GetApu()->FrameCounterTick(FrameType::HalfFrame);
					_blockFrameCounterTick = 2;
				}
			}
		}

		if(_blockFrameCounterTick > 0) {
			_blockFrameCounterTick--;
		}

		return cyclesRan;
	}

	bool NeedToRun(uint32_t cyclesToRun)
	{
		//Run APU when:
		// -A new value is pending
		// -The "blockFrameCounterTick" process is running
		// -We're at the before-last or last tick of the current step
		return _newValue >= 0 || _blockFrameCounterTick > 0 || (_previousCycle + (int32_t)cyclesToRun >= _stepCycles[_stepMode][_currentStep] - 1);
	}

	void GetMemoryRanges(MemoryRanges& ranges) override
	{
		ranges.AddHandler(MemoryOperation::Write, 0x4017);
	}

	uint8_t ReadRam(uint16_t addr) override
	{
		return 0;
	}

	void WriteRam(uint16_t addr, uint8_t value) override
	{
		_console->GetApu()->Run();
		_newValue = value;

		//Reset sequence after $4017 is written to
		if(_console->GetCpu()->GetCycleCount() & 0x01) {
			//"If the write occurs between APU cycles, the effects occur 4 CPU cycles after the write cycle. "
			_writeDelayCounter = 4;
		} else {
			//"If the write occurs during an APU cycle, the effects occur 3 CPU cycles after the $4017 write cycle"
			_writeDelayCounter = 3;
		}

		_inhibitIRQ = (value & 0x40) == 0x40;
		if(_inhibitIRQ) {
			_console->GetCpu()->ClearIrqSource(IRQSource::FrameCounter);
			_irqFlag = false;
			_irqFlagClearClock = 0;
		}
	}

	bool GetIrqFlag()
	{
		if(_irqFlag) {
			uint64_t clock = _console->GetMasterClock();
			if(_irqFlagClearClock == 0) {
				//The flag will be cleared at the start of the next APU cycle (see AccuracyCoin test)
				_irqFlagClearClock = clock + ((clock & 0x01) ? 2 : 1);
			} else if(clock >= _irqFlagClearClock) {
				_irqFlagClearClock = 0;
				_irqFlag = false;
			}
		}
		return _irqFlag;
	}

	bool PeekIrqFlag()
	{
		if(_irqFlag && _irqFlagClearClock != 0) {
			uint64_t clock = _console->GetMasterClock();
			if(clock >= _irqFlagClearClock) {
				return false;
			}
		}
		return _irqFlag;
	}

	ApuFrameCounterState GetState()
	{
		ApuFrameCounterState state;
		state.IrqEnabled = !_inhibitIRQ;
		state.SequencePosition = std::min<uint8_t>(_currentStep, _stepMode ? 5 : 4);
		state.FiveStepMode = _stepMode == 1;
		return state;
	}
};
```

"ApuLengthCounter.h"
```cpp
#pragma once
#include "pch.h"
#include "NES/NesConsole.h"
#include "NES/APU/NesApu.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"

class ApuLengthCounter : public ISerializable
{
private:
	static constexpr uint8_t _lcLookupTable[32] = { 10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30 };
	NesConsole* _console = nullptr;
	AudioChannel _channel = AudioChannel::Square1;
	bool _newHaltValue = false;

protected:
	bool _enabled = false;
	bool _halt = false;
	uint8_t _counter = 0;
	uint8_t _reloadValue = 0;
	uint8_t _previousValue = 0;

public:
	void InitializeLengthCounter(bool haltFlag)
	{
		_console->GetApu()->SetNeedToRun();
		_newHaltValue = haltFlag;
	}

	void LoadLengthCounter(uint8_t value)
	{
		if(_enabled) {
			_reloadValue = _lcLookupTable[value];
			_previousValue = _counter;
			_console->GetApu()->SetNeedToRun();
		}
	}

	ApuLengthCounter(AudioChannel channel, NesConsole* console)
	{
		_channel = channel;
		_console = console;
	}

	void Reset(bool softReset)
	{
		if(softReset) {
			_enabled = false;
			if(_channel != AudioChannel::Triangle) {
				//"At reset, length counters should be enabled, triangle unaffected"
				_halt = false;
				_counter = 0;
				_newHaltValue = false;
				_reloadValue = 0;
				_previousValue = 0;
			}
		} else {
			_enabled = false;
			_halt = false;
			_counter = 0;
			_newHaltValue = false;
			_reloadValue = 0;
			_previousValue = 0;
		}
	}

	void Serialize(Serializer& s) override
	{
		SV(_enabled);
		SV(_halt);
		SV(_newHaltValue);
		SV(_counter);
		SV(_previousValue);
		SV(_reloadValue);
	}

	bool GetStatus()
	{
		return _counter > 0;
	}

	bool IsHalted()
	{
		return _halt;
	}

	void ReloadCounter()
	{
		if(_reloadValue) {
			if(_counter == _previousValue) {
				_counter = _reloadValue;
			}
			_reloadValue = 0;
		}

		_halt = _newHaltValue;
	}

	void TickLengthCounter()
	{
		if(_counter > 0 && !_halt) {
			_counter--;
		}
	}

	void SetEnabled(bool enabled)
	{
		if(!enabled) {
			_counter = 0;
		}
		_enabled = enabled;
	}

	bool IsEnabled()
	{
		return _enabled;
	}

	ApuLengthCounterState GetState()
	{
		ApuLengthCounterState state;
		state.Counter = _counter;
		state.Halt = _halt;
		state.ReloadValue = _reloadValue;
		return state;
	}
};
```

"ApuTimer.h"
```cpp
#pragma once
#include "pch.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"
#include "NES/INesMemoryHandler.h"
#include "NES/NesConsole.h"
#include "NES/NesSoundMixer.h"

class ApuTimer : public ISerializable
{
private:
	uint32_t _previousCycle;
	uint16_t _timer = 0;
	uint16_t _period = 0;
	int8_t _lastOutput = 0;

	AudioChannel _channel = AudioChannel::Square1;
	NesSoundMixer* _mixer = nullptr;

public:
	ApuTimer(AudioChannel channel, NesSoundMixer* mixer)
	{
		_channel = channel;
		_mixer = mixer;
		Reset(false);
	}

	void Reset(bool softReset)
	{
		_timer = 0;
		_period = 0;
		_previousCycle = 0;
		_lastOutput = 0;
	}

	void Serialize(Serializer& s) override
	{
		if(!s.IsSaving()) {
			_previousCycle = 0;
		}

		SV(_timer);
		SV(_period);
		SV(_lastOutput);
	}

	__forceinline void AddOutput(int8_t output)
	{
		if(output != _lastOutput) {
			_mixer->AddDelta(_channel, _previousCycle, output - _lastOutput);
			_lastOutput = output;
		}
	}

	int8_t GetLastOutput()
	{
		return _lastOutput;
	}

	__forceinline bool Run(uint32_t targetCycle)
	{
		int32_t cyclesToRun = targetCycle - _previousCycle;

		if(cyclesToRun > _timer) {
			_previousCycle += _timer + 1;
			_timer = _period;
			return true;
		}

		_timer -= cyclesToRun;
		_previousCycle = targetCycle;
		return false;
	}

	void SetPeriod(uint16_t period)
	{
		_period = period;
	}

	uint16_t GetPeriod()
	{
		return _period;
	}

	uint16_t GetTimer()
	{
		return _timer;
	}

	void SetTimer(uint16_t timer)
	{
		_timer = timer;
	}

	__forceinline void EndFrame()
	{
		_previousCycle = 0;
	}
};
```

"DeltaModulationChannel.h"
```cpp
#pragma once
#include "pch.h"
#include "NES/APU/ApuTimer.h"
#include "NES/INesMemoryHandler.h"
#include "Utilities/ISerializable.h"

class NesConsole;

class DeltaModulationChannel : public INesMemoryHandler, public ISerializable
{
private:
	static constexpr uint16_t _dmcPeriodLookupTableNtsc[16] = { 428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54 };
	static constexpr uint16_t _dmcPeriodLookupTablePal[16] = { 398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50 };

	NesConsole* _console;
	ApuTimer _timer;

	uint16_t _sampleAddr = 0;
	uint16_t _sampleLength = 0;
	uint8_t _outputLevel = 0;
	bool _irqEnabled = false;
	bool _loopFlag = false;

	uint16_t _currentAddr = 0;
	uint16_t _bytesRemaining = 0;
	uint8_t _readBuffer = 0;
	bool _bufferEmpty = true;

	uint8_t _shiftRegister = 0;
	uint8_t _bitsRemaining = 0;
	bool _silenceFlag = true;
	bool _needToRun = false;
	uint8_t _disableDelay = 0;
	uint8_t _transferStartDelay = 0;

	uint8_t _lastValue4011 = 0;

	void InitSample();

public:
	DeltaModulationChannel(NesConsole* console);

	void Run(uint32_t targetCycle);

	void Reset(bool softReset);
	void Serialize(Serializer& s) override;

	bool IrqPending(uint32_t cyclesToRun);
	bool NeedToRun();
	bool GetStatus();
	void GetMemoryRanges(MemoryRanges& ranges) override;
	void WriteRam(uint16_t addr, uint8_t value) override;
	uint8_t ReadRam(uint16_t addr) override { return 0; }
	void EndFrame();

	void SetEnabled(bool enabled);
	void ProcessClock();
	void StartDmcTransfer();
	uint16_t GetDmcReadAddress();
	void SetDmcReadBuffer(uint8_t value);

	uint8_t GetOutput() { return _timer.GetLastOutput(); }
	ApuDmcState GetState();
};
```

"NesApu.h"
```cpp
#pragma once

#include "pch.h"
#include "Utilities/ISerializable.h"
#include "NES/INesMemoryHandler.h"
#include "NES/NesTypes.h"

class NesConsole;
class SquareChannel;
class TriangleChannel;
class NoiseChannel;
class DeltaModulationChannel;
class ApuFrameCounter;
class NesSoundMixer;
class EmuSettings;

enum class FrameType;
enum class ConsoleRegion;

class NesApu : public ISerializable, public INesMemoryHandler
{
	friend ApuFrameCounter;

private:
	bool _apuEnabled;
	bool _needToRun;

	uint32_t _previousCycle;
	uint32_t _currentCycle;

	unique_ptr<SquareChannel> _square1;
	unique_ptr<SquareChannel> _square2;
	unique_ptr<TriangleChannel> _triangle;
	unique_ptr<NoiseChannel> _noise;
	unique_ptr<DeltaModulationChannel> _dmc;
	unique_ptr<ApuFrameCounter> _frameCounter;

	NesConsole* _console;
	NesSoundMixer* _mixer;
	EmuSettings* _settings;

	ConsoleRegion _region;

	uint64_t _apuDisabledStamp = 0;

private:
	__forceinline bool NeedToRun(uint32_t currentCycle);

	void FrameCounterTick(FrameType type);

	template<bool isPeek = false>
	uint8_t GetStatus();

public:
	NesApu(NesConsole* console);
	~NesApu();

	void Serialize(Serializer& s) override;

	void Reset(bool softReset);
	void SetRegion(ConsoleRegion region, bool forceInit = false);

	uint8_t ReadRam(uint16_t addr) override;
	uint8_t PeekRam(uint16_t addr) override;
	void WriteRam(uint16_t addr, uint8_t value) override;
	void GetMemoryRanges(MemoryRanges& ranges) override;

	ApuState GetState();

	void Exec();
	void ProcessCpuClock();
	void Run();
	void EndFrame();

	void AddExpansionAudioDelta(AudioChannel channel, int16_t delta);
	void SetApuStatus(bool enabled);
	bool IsApuEnabled();
	static ConsoleRegion GetApuRegion(NesConsole* console);
	uint16_t GetDmcReadAddress();
	void SetDmcReadBuffer(uint8_t value);
	void SetNeedToRun();
};
```

"NoiseChannel.h"
```cpp
#pragma once
#include "pch.h"
#include "NES/APU/NesApu.h"
#include "NES/APU/ApuTimer.h"
#include "NES/APU/ApuEnvelope.h"
#include "NES/NesConstants.h"
#include "NES/NesConsole.h"
#include "NES/INesMemoryHandler.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"

class NoiseChannel : public INesMemoryHandler, public ISerializable
{
private:
	static constexpr uint16_t _noisePeriodLookupTableNtsc[16] = { 4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068 };
	static constexpr uint16_t _noisePeriodLookupTablePal[16] = { 4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778 };

	NesConsole* _console = nullptr;
	ApuEnvelope _envelope;
	ApuTimer _timer;

	//On power-up, the shift register is loaded with the value 1.
	uint16_t _shiftRegister = 1;
	bool _modeFlag = false;

	bool IsMuted()
	{
		//The mixer receives the current envelope volume except when Bit 0 of the shift register is set, or The length counter is zero
		return (_shiftRegister & 0x01) == 0x01;
	}

public:
	NoiseChannel(NesConsole* console) : _envelope(AudioChannel::Noise, console), _timer(AudioChannel::Noise, console->GetSoundMixer())
	{
		_console = console;
	}

	void Run(uint32_t targetCycle)
	{
		while(_timer.Run(targetCycle)) {
			//Feedback is calculated as the exclusive-OR of bit 0 and one other bit: bit 6 if Mode flag is set, otherwise bit 1.
			bool mode = _console->GetNesConfig().DisableNoiseModeFlag ? false : _modeFlag;

			uint16_t feedback = (_shiftRegister & 0x01) ^ ((_shiftRegister >> (mode ? 6 : 1)) & 0x01);
			_shiftRegister >>= 1;
			_shiftRegister |= (feedback << 14);

			if(IsMuted()) {
				_timer.AddOutput(0);
			} else {
				_timer.AddOutput(_envelope.GetVolume());
			}
		}
	}

	void TickEnvelope()
	{
		_envelope.TickEnvelope();
	}

	void TickLengthCounter()
	{
		_envelope.LengthCounter.TickLengthCounter();
	}

	void ReloadLengthCounter()
	{
		_envelope.LengthCounter.ReloadCounter();
	}

	void EndFrame()
	{
		_timer.EndFrame();
	}

	void SetEnabled(bool enabled)
	{
		_envelope.LengthCounter.SetEnabled(enabled);
	}

	bool GetStatus()
	{
		return _envelope.LengthCounter.GetStatus();
	}

	void Reset(bool softReset)
	{
		_envelope.Reset(softReset);
		_timer.Reset(softReset);

		_timer.SetPeriod((NesApu::GetApuRegion(_console) == ConsoleRegion::Ntsc ? _noisePeriodLookupTableNtsc : _noisePeriodLookupTablePal)[0] - 1);
		_shiftRegister = 1;
		_modeFlag = false;
	}

	void Serialize(Serializer& s) override
	{
		SV(_shiftRegister);
		SV(_modeFlag);
		SV(_envelope);
		SV(_timer);
	}

	void GetMemoryRanges(MemoryRanges& ranges) override
	{
		ranges.AddHandler(MemoryOperation::Write, 0x400C, 0x400F);
	}

	void WriteRam(uint16_t addr, uint8_t value) override
	{
		_console->GetApu()->Run();

		switch(addr & 0x03) {
			case 0: //400C
				_envelope.InitializeEnvelope(value);
				break;

			case 2: //400E
				_timer.SetPeriod((NesApu::GetApuRegion(_console) == ConsoleRegion::Ntsc ? _noisePeriodLookupTableNtsc : _noisePeriodLookupTablePal)[value & 0x0F] - 1);
				_modeFlag = (value & 0x80) == 0x80;
				break;

			case 3: //400F
				_envelope.LengthCounter.LoadLengthCounter(value >> 3);

				//The envelope is also restarted.
				_envelope.ResetEnvelope();
				break;
		}
	}

	uint8_t GetOutput()
	{
		return _timer.GetLastOutput();
	}

	ApuNoiseState GetState()
	{
		ApuNoiseState state;
		state.Enabled = _envelope.LengthCounter.IsEnabled();
		state.Envelope = _envelope.GetState();
		state.Frequency = (double)NesConstants::GetClockRate(NesApu::GetApuRegion(_console)) / (_timer.GetPeriod() + 1) / (_modeFlag ? 93 : 1);
		state.LengthCounter = _envelope.LengthCounter.GetState();
		state.ModeFlag = _modeFlag;
		state.OutputVolume = _timer.GetLastOutput();
		state.Period = _timer.GetPeriod();
		state.Timer = _timer.GetTimer();
		state.ShiftRegister = _shiftRegister;
		return state;
	}

	uint8_t ReadRam(uint16_t addr) override
	{
		return 0;
	}
};
```

"SquareChannel.h"
```cpp
#pragma once
#include "pch.h"
#include "NES/APU/ApuEnvelope.h"
#include "NES/APU/ApuTimer.h"
#include "NES/APU/NesApu.h"
#include "NES/NesConstants.h"
#include "NES/NesConsole.h"
#include "NES/INesMemoryHandler.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"

class SquareChannel : public INesMemoryHandler, public ISerializable
{
protected:
	static constexpr uint8_t _dutySequences[4][8] = {
		{ 0, 0, 0, 0, 0, 0, 0, 1 },
		{ 0, 0, 0, 0, 0, 0, 1, 1 },
		{ 0, 0, 0, 0, 1, 1, 1, 1 },
		{ 1, 1, 1, 1, 1, 1, 0, 0 }
	};

	NesConsole* _console = nullptr;
	ApuEnvelope _envelope;
	ApuTimer _timer;

	bool _isChannel1 = false;
	bool _isMmc5Square = false;

	uint8_t _duty = 0;
	uint8_t _dutyPos = 0;

	bool _sweepEnabled = false;
	uint8_t _sweepPeriod = 0;
	bool _sweepNegate = false;
	uint8_t _sweepShift = 0;
	bool _reloadSweep = false;
	uint8_t _sweepDivider = 0;
	uint32_t _sweepTargetPeriod = 0;
	uint16_t _realPeriod = 0;

	bool IsMuted()
	{
		//A period of t < 8, either set explicitly or via a sweep period update, silences the corresponding pulse channel.
		return _realPeriod < 8 || (!_sweepNegate && _sweepTargetPeriod > 0x7FF);
	}

	virtual void InitializeSweep(uint8_t regValue)
	{
		_sweepEnabled = (regValue & 0x80) == 0x80;
		_sweepNegate = (regValue & 0x08) == 0x08;

		//The divider's period is set to P + 1
		_sweepPeriod = ((regValue & 0x70) >> 4) + 1;
		_sweepShift = (regValue & 0x07);

		UpdateTargetPeriod();

		//Side effects: Sets the reload flag
		_reloadSweep = true;
	}

	void UpdateTargetPeriod()
	{
		uint16_t shiftResult = (_realPeriod >> _sweepShift);
		if(_sweepNegate) {
			_sweepTargetPeriod = _realPeriod - shiftResult;
			if(_isChannel1) {
				// As a result, a negative sweep on pulse channel 1 will subtract the shifted period value minus 1
				_sweepTargetPeriod--;
			}
		} else {
			_sweepTargetPeriod = _realPeriod + shiftResult;
		}
	}

	void SetPeriod(uint16_t newPeriod)
	{
		_realPeriod = newPeriod;
		_timer.SetPeriod((_realPeriod * 2) + 1);
		UpdateTargetPeriod();
	}

	void UpdateOutput()
	{
		if(IsMuted()) {
			_timer.AddOutput(0);
		} else {
			_timer.AddOutput(_dutySequences[_duty][_dutyPos] * _envelope.GetVolume());
		}
	}

public:
	SquareChannel(AudioChannel channel, NesConsole* console, bool isChannel1) : _envelope(channel, console), _timer(channel, console->GetSoundMixer())
	{
		_console = console;
		_isChannel1 = isChannel1;
	}

	void Run(uint32_t targetCycle)
	{
		while(_timer.Run(targetCycle)) {
			_dutyPos = (_dutyPos - 1) & 0x07;
			UpdateOutput();
		}
	}

	void Reset(bool softReset)
	{
		_envelope.Reset(softReset);
		_timer.Reset(softReset);

		_duty = 0;
		_dutyPos = 0;

		_realPeriod = 0;

		_sweepEnabled = false;
		_sweepPeriod = 0;
		_sweepNegate = false;
		_sweepShift = 0;
		_reloadSweep = false;
		_sweepDivider = 0;
		_sweepTargetPeriod = 0;
		UpdateTargetPeriod();
	}

	void Serialize(Serializer& s) override
	{
		SV(_realPeriod);
		SV(_duty);
		SV(_dutyPos);
		SV(_sweepEnabled);
		SV(_sweepPeriod);
		SV(_sweepNegate);
		SV(_sweepShift);
		SV(_reloadSweep);
		SV(_sweepDivider);
		SV(_sweepTargetPeriod);
		SV(_timer);
		SV(_envelope);
	}

	void GetMemoryRanges(MemoryRanges& ranges) override
	{
		if(_isChannel1) {
			ranges.AddHandler(MemoryOperation::Write, 0x4000, 0x4003);
		} else {
			ranges.AddHandler(MemoryOperation::Write, 0x4004, 0x4007);
		}
	}

	void WriteRam(uint16_t addr, uint8_t value) override
	{
		_console->GetApu()->Run();
		switch(addr & 0x03) {
			case 0: //4000 & 4004
				_envelope.InitializeEnvelope(value);

				_duty = (value & 0xC0) >> 6;
				if(_console->GetNesConfig().SwapDutyCycles && !_isMmc5Square) {
					_duty = ((_duty & 0x02) >> 1) | ((_duty & 0x01) << 1);
				}
				break;

			case 1: //4001 & 4005
				InitializeSweep(value);
				break;

			case 2: //4002 & 4006
				SetPeriod((_realPeriod & 0x0700) | value);
				break;

			case 3: //4003 & 4007
				_envelope.LengthCounter.LoadLengthCounter(value >> 3);

				SetPeriod((_realPeriod & 0xFF) | ((value & 0x07) << 8));

				//The sequencer is restarted at the first value of the current sequence.
				_dutyPos = 0;

				//The envelope is also restarted.
				_envelope.ResetEnvelope();
				break;
		}

		if(!_isMmc5Square) {
			UpdateOutput();
		}
	}

	void TickSweep()
	{
		_sweepDivider--;
		if(_sweepDivider == 0) {
			if(_sweepShift > 0 && _sweepEnabled && _realPeriod >= 8 && _sweepTargetPeriod <= 0x7FF) {
				SetPeriod(_sweepTargetPeriod);
			}
			_sweepDivider = _sweepPeriod;
		}

		if(_reloadSweep) {
			_sweepDivider = _sweepPeriod;
			_reloadSweep = false;
		}
	}

	void TickEnvelope()
	{
		_envelope.TickEnvelope();
	}

	void TickLengthCounter()
	{
		_envelope.LengthCounter.TickLengthCounter();
	}

	void ReloadLengthCounter()
	{
		_envelope.LengthCounter.ReloadCounter();
	}

	void EndFrame()
	{
		_timer.EndFrame();
	}

	void SetEnabled(bool enabled)
	{
		_envelope.LengthCounter.SetEnabled(enabled);
	}

	bool GetStatus()
	{
		return _envelope.LengthCounter.GetStatus();
	}

	uint8_t GetOutput()
	{
		return _timer.GetLastOutput();
	}

	ApuSquareState GetState()
	{
		ApuSquareState state;
		state.Duty = _duty;
		state.DutyPosition = _dutyPos;
		state.Enabled = _envelope.LengthCounter.IsEnabled();
		state.Envelope = _envelope.GetState();
		state.Frequency = NesConstants::GetClockRate(NesApu::GetApuRegion(_console)) / 16.0 / (_realPeriod + 1);
		state.LengthCounter = _envelope.LengthCounter.GetState();
		state.OutputVolume = _timer.GetLastOutput();
		state.Period = _realPeriod;
		state.Timer = _timer.GetTimer() / 2;
		state.SweepEnabled = _sweepEnabled;
		state.SweepNegate = _sweepNegate;
		state.SweepPeriod = _sweepPeriod;
		state.SweepShift = _sweepShift;
		return state;
	}

	uint8_t ReadRam(uint16_t addr) override
	{
		return 0;
	}
};
```

"TriangleChannel.h"
```cpp
#pragma once
#include "pch.h"
#include "NES/NesConsole.h"
#include "NES/NesConstants.h"
#include "NES/APU/ApuTimer.h"
#include "NES/APU/ApuLengthCounter.h"
#include "NES/INesMemoryHandler.h"
#include "Utilities/ISerializable.h"
#include "Utilities/Serializer.h"

class TriangleChannel : public INesMemoryHandler, public ISerializable
{
private:
	static constexpr uint8_t _sequence[32] = { 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15 };

	NesConsole* _console;
	ApuLengthCounter _lengthCounter;
	ApuTimer _timer;

	uint8_t _linearCounter = 0;
	uint8_t _linearCounterReload = 0;
	bool _linearReloadFlag = false;
	bool _linearControlFlag = false;

	uint8_t _sequencePosition = 0;

public:
	TriangleChannel(NesConsole* console) : _lengthCounter(AudioChannel::Triangle, console), _timer(AudioChannel::Triangle, console->GetSoundMixer())
	{
		_console = console;
	}

	void Run(uint32_t targetCycle)
	{
		while(_timer.Run(targetCycle)) {
			//The sequencer is clocked by the timer as long as both the linear counter and the length counter are nonzero.
			if(_lengthCounter.GetStatus() && _linearCounter > 0) {
				_sequencePosition = (_sequencePosition + 1) & 0x1F;

				if(_timer.GetPeriod() >= 2 || !_console->GetNesConfig().SilenceTriangleHighFreq) {
					//Disabling the triangle channel when period is < 2 removes "pops" in the audio that are caused by the ultrasonic frequencies
					//This is less "accurate" in terms of emulation, so this is an option (disabled by default)
					_timer.AddOutput(_sequence[_sequencePosition]);
				}
			}
		}
	}

	void Reset(bool softReset)
	{
		_timer.Reset(softReset);
		_lengthCounter.Reset(softReset);

		_linearCounter = 0;
		_linearCounterReload = 0;
		_linearReloadFlag = false;
		_linearControlFlag = false;

		_sequencePosition = 0;
	}

	void Serialize(Serializer& s) override
	{
		SV(_linearCounter);
		SV(_linearCounterReload);
		SV(_linearReloadFlag);
		SV(_linearControlFlag);
		SV(_sequencePosition);
		SV(_timer);
		SV(_lengthCounter);
	}

	void GetMemoryRanges(MemoryRanges& ranges) override
	{
		ranges.AddHandler(MemoryOperation::Write, 0x4008, 0x400B);
	}

	void WriteRam(uint16_t addr, uint8_t value) override
	{
		_console->GetApu()->Run();

		switch(addr & 0x03) {
			case 0: //4008
				_linearControlFlag = (value & 0x80) == 0x80;
				_linearCounterReload = value & 0x7F;

				_lengthCounter.InitializeLengthCounter(_linearControlFlag);
				break;

			case 2: //400A
				_timer.SetPeriod((_timer.GetPeriod() & 0xFF00) | value);
				break;

			case 3: //400B
				_lengthCounter.LoadLengthCounter(value >> 3);

				_timer.SetPeriod((_timer.GetPeriod() & 0xFF) | ((value & 0x07) << 8));

				//Side effects 	Sets the linear counter reload flag
				_linearReloadFlag = true;
				break;
		}
	}

	void TickLinearCounter()
	{
		if(_linearReloadFlag) {
			_linearCounter = _linearCounterReload;
		} else if(_linearCounter > 0) {
			_linearCounter--;
		}

		if(!_linearControlFlag) {
			_linearReloadFlag = false;
		}
	}

	void TickLengthCounter()
	{
		_lengthCounter.TickLengthCounter();
	}

	void ReloadLengthCounter()
	{
		_lengthCounter.ReloadCounter();
	}

	void EndFrame()
	{
		_timer.EndFrame();
	}

	void SetEnabled(bool enabled)
	{
		_lengthCounter.SetEnabled(enabled);
	}

	bool GetStatus()
	{
		return _lengthCounter.GetStatus();
	}

	uint8_t GetOutput()
	{
		return _timer.GetLastOutput();
	}

	ApuTriangleState GetState()
	{
		ApuTriangleState state;
		state.Enabled = _lengthCounter.IsEnabled();
		state.Frequency = NesConstants::GetClockRate(NesApu::GetApuRegion(_console)) / 32.0 / (_timer.GetPeriod() + 1);
		state.LengthCounter = _lengthCounter.GetState();
		state.OutputVolume = _timer.GetLastOutput();
		state.Period = _timer.GetPeriod();
		state.Timer = _timer.GetTimer();
		state.SequencePosition = _sequencePosition;
		state.LinearCounterReload = _linearCounterReload;
		state.LinearCounter = _linearCounter;
		state.LinearReloadFlag = _linearReloadFlag;
		return state;
	}

	uint8_t ReadRam(uint16_t addr) override
	{
		return 0;
	}
};
```

"DeltaModulationChannel.cpp"
```cpp
#include "pch.h"

#include "NES/APU/DeltaModulationChannel.h"
#include "NES/APU/NesApu.h"
#include "NES/NesCpu.h"
#include "NES/NesConstants.h"
#include "NES/NesConsole.h"
#include "NES/NesMemoryManager.h"

DeltaModulationChannel::DeltaModulationChannel(NesConsole* console) : _timer(AudioChannel::DMC, console->GetSoundMixer())
{
	_console = console;
}

void DeltaModulationChannel::Reset(bool softReset)
{
	_timer.Reset(softReset);

	if(!softReset) {
		//At power on, the sample address is set to $C000 and the sample length is set to 1
		//Resetting does not reset their value
		_sampleAddr = 0xC000;
		_sampleLength = 1;
	}

	_outputLevel = 0;
	_irqEnabled = false;
	_loopFlag = false;

	_currentAddr = 0;
	_bytesRemaining = 0;
	_readBuffer = 0;
	_bufferEmpty = true;

	_shiftRegister = 0;
	_bitsRemaining = 8;
	_silenceFlag = true;
	_needToRun = false;
	_transferStartDelay = 0;
	_disableDelay = 0;

	_lastValue4011 = 0;

	//Not sure if this is accurate, but it seems to make things better rather than worse (for dpcmletterbox)
	//"On the real thing, I think the power-on value is 428 (or the equivalent at least - it uses a linear feedback shift register), though only the even/oddness should matter for this test."
	_timer.SetPeriod((NesApu::GetApuRegion(_console) == ConsoleRegion::Ntsc ? _dmcPeriodLookupTableNtsc : _dmcPeriodLookupTablePal)[0] - 1);

	//Make sure the DMC doesn't tick on the first cycle - this is part of what keeps Sprite/DMC DMA tests working while fixing dmc_pitch.
	_timer.SetTimer(_timer.GetPeriod());
}

void DeltaModulationChannel::InitSample()
{
	_currentAddr = _sampleAddr;
	_bytesRemaining = _sampleLength;
	_needToRun |= _bytesRemaining > 0;
}

void DeltaModulationChannel::StartDmcTransfer()
{
	if(_bufferEmpty && _bytesRemaining > 0) {
		_console->GetCpu()->StartDmcTransfer();
	}
}

uint16_t DeltaModulationChannel::GetDmcReadAddress()
{
	return _currentAddr;
}

void DeltaModulationChannel::SetDmcReadBuffer(uint8_t value)
{
	if(_bytesRemaining > 0) {
		_readBuffer = value;
		_bufferEmpty = false;

		//"The address is incremented; if it exceeds $FFFF, it is wrapped around to $8000."
		_currentAddr++;
		if(_currentAddr == 0) {
			_currentAddr = 0x8000;
		}

		_bytesRemaining--;

		if(_bytesRemaining == 0) {
			if(_loopFlag) {
				//Looped sample should never set IRQ flag
				InitSample();
			} else if(_irqEnabled) {
				_console->GetCpu()->SetIrqSource(IRQSource::DMC);
			}
		}
	}

	//When DMA ends around the time the bit counter resets, a CPU glitch sometimes causes another DMA to be requested immediately.
	if(_bitsRemaining == 8 && _timer.GetTimer() == _timer.GetPeriod() && _console->GetNesConfig().EnableDmcSampleDuplicationGlitch) {
		//When the DMA ends on the same cycle as the bit counter resets.
		//On earlier CPUs, there is normally a 1 APU cycle gap between the end of one DMC DMA
		//and the start of another. All H CPUs and some G CPUs (those from around 1990 and later)
		//remove this gap requirement.
		_shiftRegister = _readBuffer;
		_silenceFlag = false;
		_bufferEmpty = true;

		//If the sample was 1 byte, a full DMA is performed on the same address
		//and the same sample byte is played twice in a row by the DMC.
		if(_sampleLength == 1) {
			InitSample();
		}
		StartDmcTransfer();
	} else if(_sampleLength == 1 && !_loopFlag && _bitsRemaining == 1 && _timer.GetTimer() < 2) {
		//When the DMA ends on the APU cycle before the bit counter resets.
		//If this happens right before the bit counter resets,
		//a DMA is triggered and aborted 1 cycle later (causing one halted CPU cycle)
		_shiftRegister = _readBuffer;
		_bufferEmpty = false;
		InitSample();
		_disableDelay = 3;
	}
}

void DeltaModulationChannel::Run(uint32_t targetCycle)
{
	while(_timer.Run(targetCycle)) {
		if(!_silenceFlag) {
			uint8_t bit;
			if(_console->GetNesConfig().ReverseDpcmBitOrder) {
				bit = _shiftRegister & 0x80;
				_shiftRegister <<= 1;
			} else {
				bit = _shiftRegister & 0x01;
				_shiftRegister >>= 1;
			}

			if(bit) {
				if(_outputLevel <= 125) {
					_outputLevel += 2;
				}
			} else {
				if(_outputLevel >= 2) {
					_outputLevel -= 2;
				}
			}
		}

		_bitsRemaining--;
		if(_bitsRemaining == 0) {
			_bitsRemaining = 8;
			if(_bufferEmpty) {
				_silenceFlag = true;
			} else {
				_silenceFlag = false;
				_shiftRegister = _readBuffer;
				_bufferEmpty = true;
				_needToRun = true;
				if(_transferStartDelay == 0) {
					//Don't trigger the DMA if the channel was just enabled by a 4015 write
					//The DMA will be triggered later (see ProcessClock)
					//Based on AccuracyCoin's "Delta Modulation Channel" test result
					StartDmcTransfer();
				}
			}
		}

		_timer.AddOutput(_outputLevel);
	}
}

bool DeltaModulationChannel::IrqPending(uint32_t cyclesToRun)
{
	if(_irqEnabled && _bytesRemaining > 0) {
		uint32_t cyclesToEmptyBuffer = (_bitsRemaining + (_bytesRemaining - 1) * 8) * _timer.GetPeriod();
		if(cyclesToRun >= cyclesToEmptyBuffer) {
			return true;
		}
	}
	return false;
}

bool DeltaModulationChannel::GetStatus()
{
	return _bytesRemaining > 0;
}

void DeltaModulationChannel::GetMemoryRanges(MemoryRanges& ranges)
{
	ranges.AddHandler(MemoryOperation::Write, 0x4010, 0x4013);
}

void DeltaModulationChannel::WriteRam(uint16_t addr, uint8_t value)
{
	_console->GetApu()->Run();

	switch(addr & 0x03) {
		case 0: //4010
			_irqEnabled = (value & 0x80) == 0x80;
			_loopFlag = (value & 0x40) == 0x40;

			//"The rate determines for how many CPU cycles happen between changes in the output level during automatic delta-encoded sample playback."
			//Because BaseApuChannel does not decrement when setting _timer, we need to actually set the value to 1 less than the lookup table
			_timer.SetPeriod((NesApu::GetApuRegion(_console) == ConsoleRegion::Ntsc ? _dmcPeriodLookupTableNtsc : _dmcPeriodLookupTablePal)[value & 0x0F] - 1);

			if(!_irqEnabled) {
				_console->GetCpu()->ClearIrqSource(IRQSource::DMC);
			}
			break;

		case 1: { //4011
			uint8_t newValue = value & 0x7F;
			uint8_t previousLevel = _outputLevel;
			_outputLevel = newValue;

			if(_console->GetNesConfig().ReduceDmcPopping && abs(_outputLevel - previousLevel) > 50) {
				//Reduce popping sounds for 4011 writes
				_outputLevel -= (_outputLevel - previousLevel) / 2;
			}

			//4011 applies new output right away, not on the timer's reload.  This fixes bad DMC sound when playing through 4011.
			_timer.AddOutput(_outputLevel);

			if(_lastValue4011 != value && newValue > 0) {
				_console->SetNextFrameOverclockStatus(true);
			}

			_lastValue4011 = newValue;
			break;
		}

		case 2: //4012
			_sampleAddr = 0xC000 | ((uint32_t)value << 6);
			if(value > 0) {
				_console->SetNextFrameOverclockStatus(false);
			}
			break;

		case 3: //4013
			_sampleLength = (value << 4) | 0x0001;
			if(value > 0) {
				_console->SetNextFrameOverclockStatus(false);
			}
			break;
	}
}

void DeltaModulationChannel::EndFrame()
{
	_timer.EndFrame();
}

void DeltaModulationChannel::SetEnabled(bool enabled)
{
	if(!enabled) {
		if(_disableDelay == 0) {
			//Disabling takes effect with a 1 apu cycle delay
			//If a DMA starts during this time, it gets cancelled
			//but this will still cause the CPU to be halted for 1 cycle
			if((_console->GetCpu()->GetCycleCount() & 0x01) == 0) {
				_disableDelay = 2;
			} else {
				_disableDelay = 3;
			}
		}
		_needToRun = true;
	} else if(_bytesRemaining == 0) {
		InitSample();

		//Delay a number of cycles based on odd/even cycles
		//Allows behavior to match dmc_dma_start_test
		if((_console->GetCpu()->GetCycleCount() & 0x01) == 0) {
			_transferStartDelay = 2;
		} else {
			_transferStartDelay = 3;
		}
		_needToRun = true;
	}
}

void DeltaModulationChannel::ProcessClock()
{
	if(_disableDelay && --_disableDelay == 0) {
		_disableDelay = 0;
		_bytesRemaining = 0;

		//Abort any on-going transfer that hasn't fully started
		_console->GetCpu()->StopDmcTransfer();
	}

	if(_transferStartDelay && --_transferStartDelay == 0) {
		StartDmcTransfer();
	}

	_needToRun = _disableDelay || _transferStartDelay || _bytesRemaining;
}

bool DeltaModulationChannel::NeedToRun()
{
	if(_needToRun) {
		ProcessClock();
	}
	return _needToRun;
}

ApuDmcState DeltaModulationChannel::GetState()
{
	ApuDmcState state;
	state.BytesRemaining = _bytesRemaining;
	state.IrqEnabled = _irqEnabled;
	state.Loop = _loopFlag;
	state.OutputVolume = _timer.GetLastOutput();
	state.Period = _timer.GetPeriod();
	state.Timer = _timer.GetTimer();
	state.SampleRate = (double)NesConstants::GetClockRate(NesApu::GetApuRegion(_console)) / (_timer.GetPeriod() + 1);
	state.SampleAddr = _sampleAddr;
	state.NextSampleAddr = _currentAddr;
	state.SampleLength = _sampleLength;
	return state;
}

void DeltaModulationChannel::Serialize(Serializer& s)
{
	SV(_sampleAddr);
	SV(_sampleLength);
	SV(_outputLevel);
	SV(_irqEnabled);
	SV(_loopFlag);
	SV(_currentAddr);
	SV(_bytesRemaining);
	SV(_readBuffer);
	SV(_bufferEmpty);
	SV(_shiftRegister);
	SV(_bitsRemaining);
	SV(_silenceFlag);
	SV(_needToRun);
	SV(_timer);

	SV(_transferStartDelay);
	SV(_disableDelay);
}
```

"NesApu.cpp"
```cpp
#include "pch.h"
#include "NES/APU/NesApu.h"
#include "NES/APU/SquareChannel.h"
#include "NES/APU/TriangleChannel.h"
#include "NES/APU/NoiseChannel.h"
#include "NES/APU/DeltaModulationChannel.h"
#include "NES/APU/ApuFrameCounter.h"
#include "NES/NesCpu.h"
#include "NES/NesConsole.h"
#include "NES/NesTypes.h"
#include "NES/NesMemoryManager.h"
#include "NES/NesSoundMixer.h"
#include "Shared/Emulator.h"
#include "Utilities/Serializer.h"

NesApu::NesApu(NesConsole* console)
{
	_region = ConsoleRegion::Auto;
	_apuEnabled = true;
	_needToRun = false;

	_console = console;
	_mixer = _console->GetSoundMixer();
	_settings = _console->GetEmulator()->GetSettings();

	_square1.reset(new SquareChannel(AudioChannel::Square1, _console, true));
	_square2.reset(new SquareChannel(AudioChannel::Square2, _console, false));
	_triangle.reset(new TriangleChannel(_console));
	_noise.reset(new NoiseChannel(_console));
	_dmc.reset(new DeltaModulationChannel(_console));
	_frameCounter.reset(new ApuFrameCounter(_console));

	_console->GetMemoryManager()->RegisterIODevice(_square1.get());
	_console->GetMemoryManager()->RegisterIODevice(_square2.get());
	_console->GetMemoryManager()->RegisterIODevice(_frameCounter.get());
	_console->GetMemoryManager()->RegisterIODevice(_triangle.get());
	_console->GetMemoryManager()->RegisterIODevice(_noise.get());
	_console->GetMemoryManager()->RegisterIODevice(_dmc.get());

	Reset(false);
}

NesApu::~NesApu()
{
}

void NesApu::SetRegion(ConsoleRegion region, bool forceInit)
{
	//Finish the current apu frame before switching model
	Run();
	_frameCounter->SetRegion(region);
}

void NesApu::FrameCounterTick(FrameType type)
{
	//Quarter & half frame clock envelope & linear counter
	_square1->TickEnvelope();
	_square2->TickEnvelope();
	_triangle->TickLinearCounter();
	_noise->TickEnvelope();

	if(type == FrameType::HalfFrame) {
		//Half frames clock length counter & sweep
		_square1->TickLengthCounter();
		_square2->TickLengthCounter();
		_triangle->TickLengthCounter();
		_noise->TickLengthCounter();

		_square1->TickSweep();
		_square2->TickSweep();
	}
}

template<bool isPeek>
uint8_t NesApu::GetStatus()
{
	uint8_t status = 0;
	status |= _square1->GetStatus() ? 0x01 : 0x00;
	status |= _square2->GetStatus() ? 0x02 : 0x00;
	status |= _triangle->GetStatus() ? 0x04 : 0x00;
	status |= _noise->GetStatus() ? 0x08 : 0x00;
	status |= _dmc->GetStatus() ? 0x10 : 0x00;
	if constexpr(isPeek) {
		status |= _frameCounter->PeekIrqFlag() ? 0x40 : 0x00;
	} else {
		status |= _frameCounter->GetIrqFlag() ? 0x40 : 0x00;
	}
	status |= _console->GetCpu()->HasIrqSource(IRQSource::DMC) ? 0x80 : 0x00;

	return status;
}

uint8_t NesApu::ReadRam(uint16_t addr)
{
	//$4015 read
	Run();

	if(addr >= 0x4018 && !_console->GetNesConfig().EnableCpuTestMode) {
		return _console->GetMemoryManager()->GetOpenBus();
	}

	switch(addr) {
		case 0x4015: {
			uint8_t status = GetStatus() | (_console->GetMemoryManager()->GetInternalOpenBus() & 0x20);

			//Reading $4015 clears the Frame Counter interrupt flag.
			_console->GetCpu()->ClearIrqSource(IRQSource::FrameCounter);

			return status;
		}

		case 0x4018: return _square1->GetOutput() | (_square2->GetOutput() << 4);
		case 0x4019: return _triangle->GetOutput() | (_noise->GetOutput() << 4);
		case 0x401A: return _dmc->GetOutput();

		default:
			return _console->GetMemoryManager()->GetOpenBus();
	}
}

uint8_t NesApu::PeekRam(uint16_t addr)
{
	if(_console->GetEmulator()->IsEmulationThread()) {
		//Only run the Apu (to catch up) if we're running this in the emulation thread (not 100% accurate, but we can't run the Apu from any other thread without locking)
		Run();
	}
	return GetStatus<true>();
}

void NesApu::WriteRam(uint16_t addr, uint8_t value)
{
	//$4015 write
	Run();

	//Writing to $4015 clears the DMC interrupt flag.
	//This needs to be done before setting the enabled flag for the DMC (because doing so can trigger an IRQ)
	_console->GetCpu()->ClearIrqSource(IRQSource::DMC);

	_square1->SetEnabled((value & 0x01) == 0x01);
	_square2->SetEnabled((value & 0x02) == 0x02);
	_triangle->SetEnabled((value & 0x04) == 0x04);
	_noise->SetEnabled((value & 0x08) == 0x08);
	_dmc->SetEnabled((value & 0x10) == 0x10);
}

void NesApu::GetMemoryRanges(MemoryRanges& ranges)
{
	ranges.AddHandler(MemoryOperation::Read, 0x4015);
	ranges.AddHandler(MemoryOperation::Read, 0x4018, 0x401A);
	ranges.AddHandler(MemoryOperation::Write, 0x4015);
}

void NesApu::Run()
{
	//Update framecounter and all channels
	//This is called:
	//-At the end of a frame
	//-Before Apu registers are read/written to
	//-When a DMC or FrameCounter interrupt needs to be fired
	int32_t cyclesToRun = _currentCycle - _previousCycle;

	while(cyclesToRun > 0) {
		_previousCycle += _frameCounter->Run(cyclesToRun);

		//Reload counters set by writes to 4003/4008/400B/400F after running the frame counter to allow the length counter to be clocked first
		//This fixes the test "len_reload_timing" (tests 4 & 5)
		_square1->ReloadLengthCounter();
		_square2->ReloadLengthCounter();
		_noise->ReloadLengthCounter();
		_triangle->ReloadLengthCounter();

		_square1->Run(_previousCycle);
		_square2->Run(_previousCycle);
		_noise->Run(_previousCycle);
		_triangle->Run(_previousCycle);
		_dmc->Run(_previousCycle);
	}
}

void NesApu::SetNeedToRun()
{
	_needToRun = true;
}

bool NesApu::NeedToRun(uint32_t currentCycle)
{
	if(_dmc->NeedToRun() || _needToRun) {
		//Need to run whenever we alter the length counters
		//Need to run every cycle when DMC is running to get accurate emulation (CPU stalling, interaction with sprite DMA, etc.)
		_needToRun = false;
		return true;
	}

	uint32_t cyclesToRun = currentCycle - _previousCycle;
	return _frameCounter->NeedToRun(cyclesToRun) || _dmc->IrqPending(cyclesToRun);
}

void NesApu::Exec()
{
	_currentCycle++;
	if(_currentCycle == NesSoundMixer::CycleLength - 1) {
		_dmc->ProcessClock();
		EndFrame();
	} else if(NeedToRun(_currentCycle)) {
		Run();
	}
}

void NesApu::EndFrame()
{
	Run();
	_square1->EndFrame();
	_square2->EndFrame();
	_triangle->EndFrame();
	_noise->EndFrame();
	_dmc->EndFrame();

	_mixer->PlayAudioBuffer(_currentCycle);

	_currentCycle = 0;
	_previousCycle = 0;
}

void NesApu::ProcessCpuClock()
{
	if(_apuEnabled) {
		Exec();
	}
}

void NesApu::Reset(bool softReset)
{
	_apuEnabled = true;
	_currentCycle = 0;
	_previousCycle = 0;
	_square1->Reset(softReset);
	_square2->Reset(softReset);
	_triangle->Reset(softReset);
	_noise->Reset(softReset);
	_dmc->Reset(softReset);
	_frameCounter->Reset(softReset);
}

void NesApu::Serialize(Serializer& s)
{
	if(s.GetFormat() != SerializeFormat::Map) {
		//End the Apu frame - makes it simpler to restore sound after a state reload
		EndFrame();

		SV(_apuEnabled);
		SV(_apuDisabledStamp);
	}

	SV(_square1);
	SV(_square2);
	SV(_triangle);
	SV(_noise);
	SV(_dmc);
	SV(_frameCounter);
}

void NesApu::AddExpansionAudioDelta(AudioChannel channel, int16_t delta)
{
	_mixer->AddDelta(channel, _currentCycle, delta);
}

void NesApu::SetApuStatus(bool enabled)
{
	if(_apuEnabled == enabled) {
		return;
	}

	if(!enabled) {
		_apuDisabledStamp = _console->GetCpu()->GetCycleCount();
		_apuEnabled = false;
	} else {
		uint64_t gap = _console->GetCpu()->GetCycleCount() - _apuDisabledStamp;
		if(gap & 0x01) {
			//CPU ran an odd number of cycles while the APU was disabled for overclocking
			//Run an extra APU cycle here to re-sync odd/even cycles with the CPU
			//This is needed to ensure DMC DMA occurs on the correct cycles with overclocking.
			Exec();
		}
		_apuEnabled = true;
	}
}

bool NesApu::IsApuEnabled()
{
	//Adding extra lines before/after NMI temporarely turns off the Apu
	//This appears to result in less side-effects than spreading out the Apu's
	//load over the entire PPU frame, like what was done before.
	//This is most likely due to the timing of the Frame Counter & DMC IRQs.
	return _apuEnabled;
}

ConsoleRegion NesApu::GetApuRegion(NesConsole* console)
{
	ConsoleRegion region = console->GetRegion();
	if(region == ConsoleRegion::Ntsc || region == ConsoleRegion::Dendy) {
		//Dendy APU works with NTSC timings
		return ConsoleRegion::Ntsc;
	} else {
		return region;
	}
}

uint16_t NesApu::GetDmcReadAddress()
{
	return _dmc->GetDmcReadAddress();
}

void NesApu::SetDmcReadBuffer(uint8_t value)
{
	_dmc->SetDmcReadBuffer(value);
}

ApuState NesApu::GetState()
{
	ApuState state;
	state.Dmc = _dmc->GetState();
	state.FrameCounter = _frameCounter->GetState();
	state.Noise = _noise->GetState();
	state.Square1 = _square1->GetState();
	state.Square2 = _square2->GetState();
	state.Triangle = _triangle->GetState();
	return state;
}
```