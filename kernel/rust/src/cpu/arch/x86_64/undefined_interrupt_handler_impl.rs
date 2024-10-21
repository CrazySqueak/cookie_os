
macro_rules! init_ud_body {
    (@youvegottabefuckingkiddingme, $idt:ident, $udname:ident, $vector:literal) => {
        {
            extern "x86-interrupt" fn ____handle_ud_interrupt(sf:InterruptStackFrame) {
                $udname($vector,sf)
            }
            $idt[$vector].set_handler_fn(____handle_ud_interrupt);
        }
    };
    (@youvegottabefuckingkiddingme, $idt:ident, $udname:ident,
    $v0:literal, $v1:literal, $v2:literal, $v3:literal,
    $v4:literal, $v5:literal, $v6:literal, $v7:literal,
    $v8:literal, $v9:literal, $vA:literal, $vB:literal,
    $vC:literal, $vD:literal, $vE:literal, $vF:literal $($tt:tt)*) => {
        {  // (this is manually unrolled to save on compile times + avoid hitting the recursion limit)
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v0);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v1);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v2);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v3);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v4);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v5);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v6);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v7);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v8);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$v9);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$vA);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$vB);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$vC);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$vD);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$vE);
            init_ud_body!(@youvegottabefuckingkiddingme,$idt,$udname,$vF);
        };
        init_ud_body!(@youvegottabefuckingkiddingme, $idt, $udname $($tt)*);
    };
    (@youvegottabefuckingkiddingme, $idt:ident, $udname:ident $(,)? ) => {};
}

use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::idt::InterruptStackFrame;
use super::interrupts::undefined_interrupt_handler as undefined_handler;

pub(super) fn init_undefined(idt: &mut InterruptDescriptorTable) {
    init_ud_body!(@youvegottabefuckingkiddingme, idt, undefined_handler,
        // deep inhale
        0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
        16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,
        32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,
        48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,
        64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,
        80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,
        96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,
        112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,
        128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,
        144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,
        160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,
        176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,
        192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,
        208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,
        224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,
        240,241,242,243,244,245,246,247,248,249,250,251,252,253,254,255,
        // (I'd rather list all numbers from 0..256 by hand than learn proc macros)
    );
}